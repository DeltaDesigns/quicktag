use std::{
    fmt::Display,
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use binrw::{BinReaderExt, Endian};
use destiny_pkg::{PackageManager, PackageVersion, TagHash, TagHash64};
use eframe::epaint::mutex::RwLock;
use itertools::Itertools;
use log::{error, info, warn};
use nohash_hasher::{IntMap, IntSet};
use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};

use crate::{
    packages::package_manager,
    text::create_stringmap,
    util::{u32_from_endian, u64_from_endian},
};

#[derive(serde::Serialize, serde::Deserialize)]
pub struct TagCache {
    /// Timestamp of the packages directory
    pub timestamp: u64,

    pub version: u32,

    pub hashes: IntMap<TagHash, ScanResult>,
}

impl Default for TagCache {
    fn default() -> Self {
        Self {
            timestamp: 0,
            version: 3,
            hashes: Default::default(),
        }
    }
}

// Shareable read-only context
pub struct ScannerContext {
    pub valid_file_hashes: IntSet<TagHash>,
    pub valid_file_hashes64: IntSet<TagHash64>,
    pub known_string_hashes: IntSet<u32>,
    pub endian: Endian,
}

#[derive(Clone, serde::Deserialize, serde::Serialize, Debug)]
pub struct ScanResult {
    /// Were we able to read the tag data?
    pub successful: bool,

    pub file_hashes: Vec<ScannedHash<TagHash>>,
    pub file_hashes64: Vec<ScannedHash<TagHash64>>,
    pub string_hashes: Vec<ScannedHash<u32>>,
    pub raw_strings: Vec<String>,

    /// References from other files
    pub references: Vec<TagHash>,
}

impl Default for ScanResult {
    fn default() -> Self {
        ScanResult {
            successful: true,
            file_hashes: Default::default(),
            file_hashes64: Default::default(),
            string_hashes: Default::default(),
            raw_strings: Default::default(),
            references: Default::default(),
        }
    }
}

#[derive(Clone, serde::Deserialize, serde::Serialize, Debug)]
pub struct ScannedHash<T: Sized> {
    pub offset: u64,
    pub hash: T,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct ScannedArray {
    pub offset: u64,
    pub count: usize,
    pub class: u32,
}

pub const FNV1_BASE: u32 = 0x811c9dc5;
pub const FNV1_PRIME: u32 = 0x01000193;
pub fn fnv1(data: &[u8]) -> u32 {
    data.iter().fold(FNV1_BASE, |acc, b| {
        acc.wrapping_mul(FNV1_PRIME) ^ (*b as u32)
    })
}

pub fn scan_file(context: &ScannerContext, data: &[u8]) -> ScanResult {
    let mut r = ScanResult::default();

    for (i, v) in data.chunks_exact(4).enumerate() {
        let m: [u8; 4] = v.try_into().unwrap();
        let value = u32_from_endian(context.endian, m);

        let offset = (i * 4) as u64;
        let hash = TagHash(value);

        if hash.is_pkg_file() && context.valid_file_hashes.contains(&hash) {
            r.file_hashes.push(ScannedHash { offset, hash });
        }

        // if hash.is_valid() && !hash.is_pkg_file() {
        //     r.classes.push(ScannedHash {
        //         offset,
        //         hash: value,
        //     });
        // }

        if value == 0x80800065 {
            r.raw_strings.extend(
                read_raw_string_blob(data, offset)
                    .into_iter()
                    .map(|(_, s)| s),
            );
        }

        if value != 0x811c9dc5 && context.known_string_hashes.contains(&value) {
            r.string_hashes.push(ScannedHash {
                offset,
                hash: value,
            });
        }
    }

    for (i, v) in data.chunks_exact(8).enumerate() {
        let m: [u8; 8] = v.try_into().unwrap();
        let value = u64_from_endian(context.endian, m);

        let offset = (i * 8) as u64;
        let hash = TagHash64(value);
        if context.valid_file_hashes64.contains(&hash) {
            r.file_hashes64.push(ScannedHash { offset, hash });
        }
    }

    // let mut cur = Cursor::new(data);
    // for c in &r.classes {
    //     if c.hash == 0x80809fb8 {
    //         cur.seek(SeekFrom::Start(c.offset + 4)).unwrap();

    //         let mut count_bytes = [0; 8];
    //         cur.read_exact(&mut count_bytes).unwrap();
    //         let mut class_bytes = [0; 4];
    //         cur.read_exact(&mut class_bytes).unwrap();

    //         r.arrays.push(ScannedArray {
    //             offset: c.offset + 4,
    //             count: u64::from_le_bytes(count_bytes) as usize,
    //             class: u32::from_le_bytes(class_bytes),
    //         });
    //     }
    // }

    r
}

pub fn read_raw_string_blob(data: &[u8], offset: u64) -> Vec<(u64, String)> {
    let mut strings = vec![];

    let mut c = Cursor::new(data);
    (|| {
        c.seek(SeekFrom::Start(offset + 4))?;
        let (buffer_size, buffer_base_offset) = if package_manager().version.is_d1() {
            let buffer_size: u32 = c.read_be()?;
            let buffer_base_offset = offset + 4 + 4;
            (buffer_size as u64, buffer_base_offset)
        } else {
            let buffer_size: u64 = c.read_le()?;
            let buffer_base_offset = offset + 4 + 8;
            (buffer_size, buffer_base_offset)
        };

        let mut buffer = vec![0u8; buffer_size as usize];
        c.read_exact(&mut buffer)?;

        let mut s = String::new();
        let mut string_start = 0_u64;
        for (i, b) in buffer.into_iter().enumerate() {
            match b as char {
                '\0' => {
                    if !s.is_empty() {
                        strings.push((buffer_base_offset + string_start, s.clone()));
                        s.clear();
                    }

                    string_start = i as u64 + 1;
                }
                c => s.push(c),
            }
        }

        if !s.is_empty() {
            strings.push((buffer_base_offset + string_start, s));
        }

        <anyhow::Result<()>>::Ok(())
    })()
    .ok();

    strings
}

pub fn create_scanner_context(package_manager: &PackageManager) -> anyhow::Result<ScannerContext> {
    info!("Creating scanner context");

    let endian = match package_manager.version {
        PackageVersion::DestinyTheTakenKing => Endian::Big,
        _ => Endian::Little,
    };

    let stringmap = create_stringmap()?;

    Ok(ScannerContext {
        valid_file_hashes: package_manager
            .package_entry_index
            .iter()
            .flat_map(|(pkg_id, entries)| {
                entries
                    .iter()
                    .enumerate()
                    .map(|(entry_id, _)| TagHash::new(*pkg_id, entry_id as _))
                    .collect_vec()
            })
            .collect(),
        valid_file_hashes64: package_manager
            .hash64_table
            .keys()
            .map(|&v| TagHash64(v))
            .collect(),
        known_string_hashes: stringmap.keys().cloned().collect(),
        endian,
    })
}

#[derive(Copy, Clone)]
pub enum ScanStatus {
    None,
    CreatingScanner,
    Scanning {
        current_package: usize,
        total_packages: usize,
    },
    TransformGathering,
    TransformApplying,
    WritingCache,
    LoadingCache,
}

impl Display for ScanStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanStatus::None => Ok(()),
            ScanStatus::CreatingScanner => f.write_str("Creating scanner"),
            ScanStatus::Scanning {
                current_package,
                total_packages,
            } => f.write_fmt(format_args!(
                "Creating new cache {}/{}",
                current_package + 1,
                total_packages
            )),
            ScanStatus::TransformGathering => {
                f.write_str("Transforming cache (gathering references)")
            }
            ScanStatus::TransformApplying => {
                f.write_str("Transforming cache (applying references)")
            }
            ScanStatus::WritingCache => f.write_str("Writing cache"),
            ScanStatus::LoadingCache => f.write_str("Loading cache"),
        }
    }
}

lazy_static::lazy_static! {
    static ref SCANNER_PROGRESS: RwLock<ScanStatus> = RwLock::new(ScanStatus::None);
}

/// Returns Some((current_package, total_packages)) if there's a scan in progress
pub fn scanner_progress() -> ScanStatus {
    *SCANNER_PROGRESS.read()
}

pub fn load_tag_cache(version: PackageVersion) -> TagCache {
    let cache_name = format!("tags_{}.cache", version.id());
    let cache_file_path = exe_relative_path(&cache_name);

    if let Ok(cache_file) = File::open(&cache_file_path) {
        info!("Existing cache file found, loading");
        *SCANNER_PROGRESS.write() = ScanStatus::LoadingCache;

        match zstd::Decoder::new(cache_file) {
            Ok(zstd_decoder) => {
                if let Ok(cache) = bincode::deserialize_from::<_, TagCache>(zstd_decoder) {
                    match cache.version.cmp(&TagCache::default().version) {
                        std::cmp::Ordering::Equal => {
                            let current_pkg_timestamp =
                                std::fs::metadata(&package_manager().package_dir)
                                    .ok()
                                    .and_then(|m| {
                                        Some(
                                            m.modified()
                                                .ok()?
                                                .duration_since(SystemTime::UNIX_EPOCH)
                                                .ok()?
                                                .as_secs(),
                                        )
                                    })
                                    .unwrap_or(0);

                            if cache.timestamp < current_pkg_timestamp {
                                info!(
                                    "Cache is out of date, rebuilding (cache: {}, package dir: {})",
                                    chrono::NaiveDateTime::from_timestamp_opt(
                                        cache.timestamp as i64,
                                        0
                                    )
                                    .unwrap()
                                    .format("%Y-%m-%d"),
                                    chrono::NaiveDateTime::from_timestamp_opt(
                                        current_pkg_timestamp as i64,
                                        0
                                    )
                                    .unwrap()
                                    .format("%Y-%m-%d"),
                                );
                            } else {
                                *SCANNER_PROGRESS.write() = ScanStatus::None;
                                return cache;
                            }
                        }
                        std::cmp::Ordering::Less => {
                            info!(
                                "Cache is out of date, rebuilding (cache: {}, quicktag: {})",
                                cache.version,
                                TagCache::default().version
                            );
                        }
                        std::cmp::Ordering::Greater => {
                            error!("Tried to open a future version cache with an old quicktag version (cache: {}, quicktag: {})",
                                cache.version,
                                TagCache::default().version
                            );

                            native_dialog::MessageDialog::new()
                                .set_type(native_dialog::MessageType::Error)
                                .set_title("Future cache")
                                .set_text(&format!("Your cache file ({cache_name}) is newer than this build of quicktag\n\nCache version: v{}\nExpected version: v{}", cache.version, TagCache::default().version))
                                .show_alert()
                                .unwrap();

                            std::process::exit(21);
                        }
                    }
                } else {
                    warn!("Cache file is invalid, creating a new one");
                }
            }
            Err(e) => error!("Cache file is invalid: {e}"),
        }
    }

    *SCANNER_PROGRESS.write() = ScanStatus::CreatingScanner;
    let scanner_context = Arc::new(
        create_scanner_context(&package_manager()).expect("Failed to create scanner context"),
    );

    let all_pkgs = package_manager()
        .package_paths
        .values()
        .cloned()
        .collect_vec();

    let package_count = all_pkgs.len();
    let cache: IntMap<TagHash, ScanResult> = all_pkgs
        .par_iter()
        .map_with(scanner_context, |context, path| {
            let current_package = {
                let mut p = SCANNER_PROGRESS.write();
                let current_package = if let ScanStatus::Scanning {
                    current_package, ..
                } = *p
                {
                    current_package
                } else {
                    0
                };

                *p = ScanStatus::Scanning {
                    current_package: current_package + 1,
                    total_packages: package_count,
                };

                current_package
            };
            info!("Opening pkg {path} ({}/{package_count})", current_package);
            let pkg = version.open(path).unwrap();

            let mut all_tags = if version.is_d1() {
                [pkg.get_all_by_type(0, None)].concat()
            } else {
                pkg.get_all_by_type(8, None)
                    .iter()
                    .chain(pkg.get_all_by_type(16, None).iter())
                    .cloned()
                    .collect_vec()
            };

            // Sort tags by entry index to optimize sequential block reads
            all_tags.sort_by_key(|v| v.0);

            let mut results = IntMap::default();
            for (t, _) in all_tags {
                let hash = TagHash::new(pkg.pkg_id(), t as u16);

                let data = match pkg.read_entry(t) {
                    Ok(d) => d,
                    Err(e) => {
                        error!("Failed to read entry {path}:{t}: {e}");
                        results.insert(
                            hash,
                            ScanResult {
                                successful: false,
                                ..Default::default()
                            },
                        );
                        continue;
                    }
                };

                let mut scan_result = scan_file(context, &data);
                if version.is_d1() {
                    if let Some(entry) = pkg.entry(t) {
                        let ref_tag = TagHash(entry.reference);
                        if context.valid_file_hashes.contains(&ref_tag) {
                            scan_result.file_hashes.insert(
                                0,
                                ScannedHash {
                                    offset: u64::MAX,
                                    hash: ref_tag,
                                },
                            );
                        }
                    }
                }
                results.insert(hash, scan_result);
            }

            results
        })
        .flatten()
        .collect();

    // panic!("{:?}", cache[&TagHash(u32::from_be(0x00408180))]);

    let cache = transform_tag_cache(cache);

    *SCANNER_PROGRESS.write() = ScanStatus::WritingCache;
    info!("Serializing tag cache...");
    let cache_bincode = bincode::serialize(&cache).unwrap();
    info!("Compressing tag cache...");
    let mut writer = zstd::Encoder::new(File::create(cache_file_path).unwrap(), 5).unwrap();
    writer.write_all(&cache_bincode).unwrap();
    writer.finish().unwrap();
    *SCANNER_PROGRESS.write() = ScanStatus::None;

    // for (t, r) in &cache {
    //     if matches!(t.pkg_id(), 0x3ac | 0x3da | 0x3db) {
    //         println!(
    //             "{} {t} {}",
    //             package_manager().package_paths.get(&t.pkg_id()).unwrap(),
    //             r.references.iter().map(TagHash::to_string).join(", ")
    //         );
    //     }
    // }

    cache
}

/// Transforms the tag cache to include reference lookup tables
fn transform_tag_cache(cache: IntMap<TagHash, ScanResult>) -> TagCache {
    info!("Transforming tag cache...");

    let mut new_cache: TagCache = Default::default();

    *SCANNER_PROGRESS.write() = ScanStatus::TransformGathering;
    info!("\t- Gathering references");
    let mut direct_reference_cache: IntMap<TagHash, Vec<TagHash>> = Default::default();
    for (k2, v2) in &cache {
        for t32 in &v2.file_hashes {
            match direct_reference_cache.entry(t32.hash) {
                std::collections::hash_map::Entry::Occupied(mut o) => {
                    o.get_mut().push(*k2);
                }
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert(vec![*k2]);
                }
            }
        }

        for t64 in &v2.file_hashes64 {
            if let Some(t32) = package_manager().hash64_table.get(&t64.hash.0) {
                match direct_reference_cache.entry(t32.hash32) {
                    std::collections::hash_map::Entry::Occupied(mut o) => {
                        o.get_mut().push(*k2);
                    }
                    std::collections::hash_map::Entry::Vacant(v) => {
                        v.insert(vec![*k2]);
                    }
                }
            }
        }
    }

    *SCANNER_PROGRESS.write() = ScanStatus::TransformApplying;
    info!("\t- Applying references");
    for (k, v) in &cache {
        let mut scan = v.clone();

        if let Some(refs) = direct_reference_cache.get(k) {
            scan.references = refs.clone();
        }

        new_cache.hashes.insert(*k, scan);
    }

    info!("\t- Adding remaining non-structure tags");
    for (k, v) in direct_reference_cache {
        if !v.is_empty() && !new_cache.hashes.contains_key(&k) {
            new_cache.hashes.insert(
                k,
                ScanResult {
                    references: v,
                    ..Default::default()
                },
            );
        }
    }

    let timestamp = std::fs::metadata(&package_manager().package_dir)
        .ok()
        .and_then(|m| {
            Some(
                m.modified()
                    .ok()?
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .ok()?
                    .as_secs(),
            )
        })
        .unwrap_or(0);

    new_cache.timestamp = timestamp;

    new_cache
}

fn exe_directory() -> PathBuf {
    std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn exe_relative_path<P: AsRef<Path>>(path: P) -> PathBuf {
    exe_directory().join(path.as_ref())
}
