// TODO(cohae): This is all copied from alkahest, and needs to be moved into alkahest-data when it becomes available

use std::fmt::{Debug, Formatter, Write};
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::ops::Deref;
use std::slice::Iter;

use binrw::{BinRead, BinReaderExt, BinResult, Endian};
use destiny_pkg::{PackageVersion, TagHash};
use eframe::epaint::ahash::HashSet;
use log::warn;
use nohash_hasher::IntMap;

use crate::packages::package_manager;

pub type TablePointer32<T> = _TablePointer<i32, u32, T>;
pub type TablePointer64<T> = _TablePointer<i64, u64, T>;
pub type TablePointer<T> = TablePointer64<T>;

pub type RelPointer32<T = ()> = _RelPointer<i32, T>;
pub type RelPointer64<T = ()> = _RelPointer<i64, T>;
pub type RelPointer<T = ()> = RelPointer64<T>;

#[derive(Clone)]
pub struct _TablePointer<O: Into<i64>, C: Into<u64>, T: BinRead> {
    offset_base: u64,
    offset: O,
    count: C,

    data: Vec<T>,
}

impl<'a, O: Into<i64>, C: Into<u64>, T: BinRead> BinRead for _TablePointer<O, C, T>
where
    C: BinRead + Copy,
    O: BinRead + Copy,
    C::Args<'a>: Default + Clone,
    O::Args<'a>: Default + Clone,
    T::Args<'a>: Default + Clone,
{
    type Args<'b> = ();

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        endian: Endian,
        _args: Self::Args<'_>,
    ) -> BinResult<Self> {
        let count: C = reader.read_type(endian)?;
        let offset_base = reader.stream_position()?;
        let offset: O = reader.read_type(endian)?;

        let offset_save = reader.stream_position()?;

        let seek64: i64 = offset.into();
        reader.seek(SeekFrom::Start(offset_base))?;
        reader.seek(SeekFrom::Current(seek64 + 16))?;

        let count64: u64 = count.into();
        let mut data = Vec::with_capacity(count64 as _);
        for _ in 0..count64 {
            data.push(reader.read_type(endian)?);
        }

        reader.seek(SeekFrom::Start(offset_save))?;

        Ok(_TablePointer {
            offset_base,
            offset,
            count,
            data,
        })
    }
}

impl<O: Into<i64> + Copy, C: Into<u64> + Copy, T: BinRead> _TablePointer<O, C, T> {
    pub fn iter(&self) -> Iter<'_, T> {
        self.data.iter()
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn data(&self) -> &[T] {
        &self.data
    }
}

impl<O: Into<i64> + Copy, C: Into<u64> + Copy, T: BinRead> Deref for _TablePointer<O, C, T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<'a, O: Into<i64> + Copy, C: Into<u64> + Copy, T: BinRead> IntoIterator
    for &'a _TablePointer<O, C, T>
{
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.data.iter()
    }
}

impl<O: Into<i64> + Copy, C: Into<u64> + Copy, T: BinRead + Debug> Debug
    for _TablePointer<O, C, T>
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "TablePointer(address=0x{:x}, count={}",
            self.offset_base as i64 + self.offset.into(),
            self.count.into(),
        ))?;

        f.write_str(", data=")?;
        self.data.fmt(f)?;

        f.write_char(')')
    }
}

#[derive(Clone, Copy)]
pub struct _RelPointer<O: Into<i64>, T: BinRead> {
    offset_base: u64,
    offset: O,

    data: T,
}

impl<'a, O: Into<i64>, T: BinRead> BinRead for _RelPointer<O, T>
where
    O: BinRead + Copy,
    O::Args<'a>: Default + Clone,
    T::Args<'a>: Default + Clone,
{
    type Args<'b> = ();

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        endian: Endian,
        _args: Self::Args<'_>,
    ) -> BinResult<Self> {
        let offset_base = reader.stream_position()?;
        let offset: O = reader.read_type(endian)?;

        let offset_save = reader.stream_position()?;

        let seek64: i64 = offset.into();
        reader.seek(SeekFrom::Start(offset_base))?;
        reader.seek(SeekFrom::Current(seek64))?;

        let data = reader.read_type(endian)?;

        reader.seek(SeekFrom::Start(offset_save))?;

        Ok(_RelPointer {
            offset_base,
            offset,
            data,
        })
    }
}

impl<O: Into<i64> + Copy, T: BinRead> Deref for _RelPointer<O, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<O: Into<i64> + Copy, T: BinRead + Debug> Debug for _RelPointer<O, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "RelPointer(address=0x{:x}",
            self.offset_base as i64 + self.offset.into(),
        ))?;

        f.write_str(", data=")?;
        self.data.fmt(f)?;

        f.write_char(')')
    }
}

impl<O: Into<i64> + Copy, T: BinRead + Debug> From<_RelPointer<O, T>> for SeekFrom {
    fn from(val: _RelPointer<O, T>) -> Self {
        SeekFrom::Start((val.offset_base as i64 + val.offset.into()) as u64)
    }
}

#[derive(BinRead, Debug)]
pub struct StringContainer {
    pub file_size: u64,
    pub string_hashes: TablePointer<u32>,
    pub language_english: TagHash,
    // pub language_unk1: TagHash,
    // pub language_german: TagHash,
    // pub language_french: TagHash,
    // pub language_unk4: TagHash,
    // pub language_unk5: TagHash,
    // pub language_italian: TagHash,
    // pub language_unk7: TagHash,
    // pub language_unk8: TagHash,
    // pub language_unk9: TagHash,
    // pub language_unk10: TagHash,
    // pub language_polish: TagHash,
    // pub language_unk12: TagHash,
}

#[derive(BinRead, Debug)]
#[br(import(prebl: bool))]
pub struct StringData {
    pub file_size: u64,
    pub string_parts: TablePointer<StringPart>,
    #[br(if(prebl))]
    pub _unk1: (u64, u64),
    pub _unk2: TablePointer<()>,
    pub string_data: TablePointer<u8>,
    pub string_combinations: TablePointer<StringCombination>,
}

#[derive(BinRead, Debug)]
pub struct StringCombination {
    pub data: RelPointer,
    pub part_count: i64,
}

#[derive(BinRead, Debug)]
pub struct StringPart {
    pub _unk0: u64,
    pub data: RelPointer,
    pub _unk1: u32,

    /// String data length.
    /// This is always equal to or larger than the string length due to variable character lengths
    pub byte_length: u16,
    pub string_length: u16,
    pub cipher_shift: u16,

    pub _unk2: u16,
    pub _unk3: u32,
}

#[derive(BinRead, Debug)]
pub struct StringContainerD1 {
    pub file_size: u32,
    pub string_hashes: TablePointer32<u32>,
    pub language_english: TagHash,
}

#[derive(BinRead, Debug)]
pub struct StringDataD1 {
    #[br(assert(file_size < 0xf00000))]
    pub file_size: u32,
    pub string_parts: TablePointer32<StringPartD1>,
    pub _unk1: (u32, u32),
    pub _unk2: TablePointer32<()>,
    pub string_data: TablePointer32<u8>,
    pub string_combinations: TablePointer32<StringCombinationD1>,
}

#[derive(BinRead, Debug)]
pub struct StringPartD1 {
    pub _unk0: u32,
    pub data: RelPointer32,
    pub _unk1: u32,

    pub byte_length: u16,
    pub string_length: u16,
    pub cipher_shift: u16,
    pub _unk2: u16,
}

#[derive(BinRead, Debug)]
pub struct StringCombinationD1 {
    pub data: RelPointer32,
    pub part_count: i32,
}

/// Expects raw un-shifted data as input
pub fn decode_text(data: &[u8], cipher: u16) -> String {
    // cohae: Modern versions of D2 no longer use the cipher system, we can take a shortcut
    if cipher == 0 {
        return String::from_utf8_lossy(data).to_string();
    }

    let mut data_clone = data.to_vec();

    let mut off = 0;
    // TODO(cohae): Shifting doesn't work entirely yet, there's still some weird characters beyond starting byte 0xe0
    while off < data.len() {
        match data[off] {
            0..=0xbf => {
                data_clone[off] += cipher as u8;
                off += 1
            }
            0xc0..=0xdf => {
                data_clone[off + 1] += cipher as u8;
                off += 2
            }
            0xe0..=0xef => {
                data_clone[off + 2] += cipher as u8;
                off += 3
            }
            0xf0..=0xff => {
                data_clone[off + 3] += cipher as u8;
                off += 4
            }
        }
    }

    String::from_utf8_lossy(&data_clone).to_string()
}

pub fn create_stringmap() -> anyhow::Result<StringCache> {
    if matches!(
        package_manager().version,
        PackageVersion::Destiny2Shadowkeep
            | PackageVersion::Destiny2BeyondLight
            | PackageVersion::Destiny2WitchQueen
            | PackageVersion::Destiny2Lightfall
    ) {
        create_stringmap_d2()
    } else if package_manager().version == PackageVersion::DestinyTheTakenKing {
        create_stringmap_d1()
    } else {
        warn!(
            "{:?} does not support string loading",
            package_manager().version
        );
        Ok(StringCache::default())
    }
}

pub fn create_stringmap_d2() -> anyhow::Result<StringCache> {
    let prebl = package_manager().version == PackageVersion::Destiny2Shadowkeep;
    let mut tmp_map: IntMap<u32, HashSet<String>> = Default::default();
    for (t, _) in package_manager()
        .get_all_by_reference(u32::from_be(if prebl { 0x889a8080 } else { 0xEF998080 }))
        .into_iter()
    {
        let Ok(textset_header) = package_manager().read_tag_struct::<StringContainer>(t) else {
            continue;
        };

        let Ok(data) = package_manager().read_tag(textset_header.language_english) else {
            continue;
        };
        let mut cur = Cursor::new(&data);
        let text_data: StringData = cur.read_le_args((prebl,))?;

        for (combination, hash) in text_data
            .string_combinations
            .iter()
            .zip(textset_header.string_hashes.iter())
        {
            let mut final_string = String::new();

            for ip in 0..combination.part_count {
                cur.seek(combination.data.into())?;
                cur.seek(SeekFrom::Current(ip * 0x20))?;
                let part: StringPart = cur.read_le()?;
                cur.seek(part.data.into())?;
                let mut data = vec![0u8; part.byte_length as usize];
                cur.read_exact(&mut data)?;
                final_string += &decode_text(&data, part.cipher_shift);
            }

            tmp_map.entry(*hash).or_default().insert(final_string);
        }
    }

    Ok(tmp_map
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect())
}

pub fn create_stringmap_d1() -> anyhow::Result<StringCache> {
    let mut tmp_map: IntMap<u32, HashSet<String>> = Default::default();
    for (t, _) in package_manager()
        .get_all_by_reference(0x8080035a)
        .into_iter()
    {
        let Ok(textset_header) = package_manager().read_tag_struct::<StringContainerD1>(t) else {
            continue;
        };

        let Ok(data) = package_manager().read_tag(textset_header.language_english) else {
            continue;
        };
        let mut cur = Cursor::new(&data);
        let Ok(text_data) = cur.read_be::<StringDataD1>() else {
            continue;
        };

        for (combination, hash) in text_data
            .string_combinations
            .iter()
            .zip(textset_header.string_hashes.iter())
        {
            if *hash == 0x811c9dc5 {
                continue;
            }

            let mut final_string = String::new();

            for ip in 0..combination.part_count {
                cur.seek(combination.data.into())?;
                cur.seek(SeekFrom::Current((ip as i64) * 20))?;
                let part: StringPartD1 = cur.read_be()?;
                cur.seek(part.data.into())?;
                let mut data = vec![0u8; part.byte_length as usize];
                cur.read_exact(&mut data)?;
                final_string += &decode_text(&data, part.cipher_shift);
            }

            tmp_map.entry(*hash).or_default().insert(final_string);
        }
    }

    Ok(tmp_map
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect())
}

pub type StringCache = IntMap<u32, Vec<String>>;
pub type StringCacheVec = Vec<(u32, Vec<String>)>;
