//! Minimal read-only FAT16/FAT32 layer for files on [`sd`] (or any other
//! block device). Built fresh for gc-rust (libogc's core doesn't ship one;
//! libfat is a different project) — deliberately small: MBR or superfloppy,
//! short 8.3 names only, whole-file reads into a caller buffer.
//!
//! Wrapped as a [`BlockIo`] trait like [`card::CardBus`] so host tests run
//! the very same code against crafted images.

/// Sector reader (512-byte units).
use alloc::vec::Vec;

pub trait BlockIo {
    fn read_block(&mut self, lba: u32, buf: &mut [u8; 512]) -> i32;
}

pub const FAT_ERROR_OK: i32 = 0;
pub const FAT_ERROR_NOFS: i32 = -20;
pub const FAT_ERROR_NOFILE: i32 = -21;
pub const FAT_ERROR_TOOLONG: i32 = -22;
pub const FAT_ERROR_IO: i32 = -23;
pub const FAT_ERROR_NOSPACE: i32 = -24;

/// A mounted filesystem.
pub struct Fat<'io> {
    io: &'io mut dyn BlockIo,
    // BPB decoded
    fat_type: FatType,
    sectors_per_cluster: u32,
    #[allow(dead_code)]
    reserved_sectors: u32,
    #[allow(dead_code)]
    num_fats: u32,
    root_entries: u32,      // 0 for FAT32
    #[allow(dead_code)]
    sectors_per_fat: u32,
    first_data_lba: u32,    // lba of cluster 2
    first_fat_lba: u32,
    root_dir_lba: u32,      // FAT16: lba of root dir; FAT32: cluster of root
    #[allow(dead_code)]
    part_lba: u32,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum FatType {
    Fat16,
    Fat32,
}

#[derive(Clone, Debug)]
pub struct DirEntry {
    pub name: [u8; 11],
    pub is_dir: bool,
    pub size: u32,
    pub first_cluster: u32,
}

impl DirEntry {
    /// Short filename, e.g. `BOOT    TXT` -> "BOOT.TXT".
    pub fn name_str(&self) -> [u8; 12] {
        let mut out = [0u8; 12];
        let mut w = 0usize;
        for i in 0..8 {
            if self.name[i] == b' ' {
                break;
            }
            out[w] = self.name[i];
            w += 1;
        }
        if self.name[8] != b' ' {
            out[w] = b'.';
            w += 1;
            for i in 8..11 {
                if self.name[i] == b' ' {
                    break;
                }
                out[w] = self.name[i];
                w += 1;
            }
        }
        // trim trailing dot if no ext
        if w > 0 && out[w - 1] == b'.' {
            w -= 1;
        }
        out[w.min(11)] = 0;
        out
    }
}

fn get16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn get32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Normalize a path component to an uppercased 8.3 SFN token.
fn to_sfn(name: &str) -> Option<[u8; 11]> {
    let mut out = [b' '; 11];
    let stripped = name.trim_matches(['/', ' ']);
    let (base, ext) = match stripped.find('.') {
        Some(d) => (&stripped[..d], &stripped[d + 1..]),
        None => (stripped, ""),
    };
    if base.is_empty() || base.len() > 8 || ext.len() > 3 {
        return None;
    }
    for (i, c) in base.bytes().enumerate() {
        out[i] = c.to_ascii_uppercase();
    }
    for (i, c) in ext.bytes().enumerate() {
        out[8 + i] = c.to_ascii_uppercase();
    }
    Some(out)
}

impl<'io> Fat<'io> {
    /// Mount the filesystem from `io` (MBR or superfloppy).
    pub fn mount(io: &'io mut dyn BlockIo) -> Result<Fat<'io>, i32> {
        let mut sec = [0u8; 512];
        if io.read_block(0, &mut sec) != 0 {
            return Err(FAT_ERROR_IO);
        }
        let mut part_lba = 0u32;
        if sec[510] != 0x55 || sec[511] != 0xAA {
            return Err(FAT_ERROR_NOFS);
        }
        // MBR? (partition table at 0x1BE with type bytes)
        let mut bpb_lba = 0u32;
        for i in 0..4 {
            let pe = 0x1be + i * 16;
            let ptype = sec[pe + 4];
            let lba = get32(&sec, pe + 8);
            if matches!(ptype, 0x0B | 0x0C | 0x0E | 0x06 | 0x04) && lba > 0 {
                bpb_lba = lba;
                part_lba = lba;
                break;
            }
        }
        if bpb_lba != 0 {
            if io.read_block(bpb_lba, &mut sec) != 0 {
                return Err(FAT_ERROR_IO);
            }
        }

        if sec[510] != 0x55 || sec[511] != 0xAA {
            return Err(FAT_ERROR_NOFS);
        }
        let bytes_per_sector = get16(&sec, 11);
        if bytes_per_sector != 512 {
            return Err(FAT_ERROR_NOFS);
        }
        let sectors_per_cluster = sec[13] as u32;
        let reserved = get16(&sec, 14) as u32;
        let num_fats = sec[16] as u32;
        let root_entries = get16(&sec, 17) as u32;
        let total16 = get16(&sec, 19) as u32;
        let _media = sec[21];
        let fat16_size = get16(&sec, 22) as u32;
        let total32 = get32(&sec, 32);
        let fat32_size = get32(&sec, 36);
        let root_cluster = get32(&sec, 44);

        let sectors_per_fat = if fat16_size != 0 { fat16_size } else { fat32_size };
        if sectors_per_fat == 0 {
            return Err(FAT_ERROR_NOFS);
        }
        let total = if total16 != 0 { total16 } else { total32 };
        let root_dir_sectors = (root_entries * 32 + 511) / 512;
        let first_data_lba = bpb_lba + reserved + num_fats * sectors_per_fat + root_dir_sectors;
        if first_data_lba >= bpb_lba + total {
            return Err(FAT_ERROR_NOFS);
        }
        let cluster_count = (bpb_lba + total - first_data_lba) / sectors_per_cluster;
        let fat_type = if cluster_count < 65525 { FatType::Fat16 } else { FatType::Fat32 };
        if fat_type == FatType::Fat16 && root_entries == 0 {
            return Err(FAT_ERROR_NOFS);
        }

        Ok(Fat {
            io,
            fat_type,
            sectors_per_cluster,
            reserved_sectors: reserved,
            num_fats,
            root_entries,
            sectors_per_fat,
            first_data_lba,
            first_fat_lba: bpb_lba + reserved,
            root_dir_lba: if fat_type == FatType::Fat16 {
                bpb_lba + reserved + num_fats * sectors_per_fat
            } else {
                root_cluster
            },
            part_lba,
        })
    }

    #[inline]
    fn cluster_lba(&self, cluster: u32) -> u32 {
        self.first_data_lba + (cluster - 2) * self.sectors_per_cluster
    }

    /// One FAT entry: next cluster (or end-of-chain marker).
    fn fat_next(&mut self, cluster: u32) -> Result<u32, i32> {
        let mut sec = [0u8; 512];
        match self.fat_type {
            FatType::Fat16 => {
                let off = cluster * 2;
                let lba = self.first_fat_lba + off / 512;
                if self.io.read_block(lba, &mut sec) != 0 {
                    return Err(FAT_ERROR_IO);
                }
                Ok(get16(&sec, (off % 512) as usize) as u32)
            }
            FatType::Fat32 => {
                let off = cluster * 4;
                let lba = self.first_fat_lba + off / 512;
                if self.io.read_block(lba, &mut sec) != 0 {
                    return Err(FAT_ERROR_IO);
                }
                Ok(get32(&sec, (off % 512) as usize) & 0x0fff_ffff)
            }
        }
    }

    #[doc(hidden)]
    fn is_eoc(&self, cluster: u32) -> bool {
        match self.fat_type {
            FatType::Fat16 => cluster >= 0xFFF8,
            FatType::Fat32 => cluster >= 0x0FFF_FFF8,
        }
    }

    /// Read the next cluster of a chain into `buf` (must be
    /// `sectors_per_cluster*512` bytes long).
    fn read_cluster(&mut self, cluster: u32, buf: &mut [u8]) -> i32 {
        let lba = self.cluster_lba(cluster);
        for i in 0..self.sectors_per_cluster {
            let start = i as usize * 512;
            let chunk: &mut [u8; 512] = match (&mut buf[start..start + 512]).try_into() {
                Ok(c) => c,
                Err(_) => return FAT_ERROR_IO,
            };
            if self.io.read_block(lba + i, chunk) != 0 {
                return FAT_ERROR_IO;
            }
        }
        0
    }

    fn read_root_fat16_entries(&mut self, out: &mut Vec<DirEntry>) -> i32 {
        let mut lba = self.root_dir_lba;
        for _ in 0..(self.root_entries * 32 + 511) / 512 {
            let mut sec = [0u8; 512];
            if self.io.read_block(lba, &mut sec) != 0 {
                return FAT_ERROR_IO;
            }
            self.parse_dir_sector(&sec, out);
            lba += 1;
        }
        0
    }

    fn parse_dir_sector(&mut self, sec: &[u8; 512], out: &mut Vec<DirEntry>) {
        for i in 0..16 {
            let e = &sec[i * 32..i * 32 + 32];
            if e[0] == 0x00 {
                break;
            }
            if e[0] == 0xE5 || e[11] == 0x0F {
                continue; // deleted / LFN fragment
            }
            if e[11] & 0x08 != 0 {
                continue; // volume label
            }
            let mut name = [b' '; 11];
            name.copy_from_slice(&e[0..11]);
            let is_dir = e[11] & 0x10 != 0;
            let first_cluster = (get16(e, 26) as u32) | ((get16(e, 20) as u32) << 16);
            let size = get32(e, 28);
            out.push(DirEntry { name, is_dir, size, first_cluster });
        }
    }

    /// List one directory level (single path component only, e.g. "" or "/").
    pub fn list(&mut self) -> Result<Vec<DirEntry>, i32> {
        let mut out = Vec::new();
        if self.fat_type == FatType::Fat16 {
            self.read_root_fat16_entries(&mut out);
            return Ok(out);
        }
        let mut cluster = self.root_dir_lba;
        loop {
            let lba = self.cluster_lba(cluster);
            let mut sec = [0u8; 512];
            for s in 0..self.sectors_per_cluster {
                if self.io.read_block(lba + s, &mut sec) != 0 {
                    return Err(FAT_ERROR_IO);
                }
                self.parse_dir_sector(&sec, &mut out);
            }
            let next = self.fat_next(cluster)?;
            if self.is_eoc(next) {
                break;
            }
            cluster = next;
        }
        Ok(out)
    }

    /// Read a *root-level* file (`"NAME.EXT"` ≤ 8.3). Subdir support is
    /// intentionally omitted (first version); iterate with [`list`] first.
    pub fn read_file(&mut self, name: &str, buf: &mut [u8]) -> Result<u32, i32> {
        let sfn = to_sfn(name).ok_or(FAT_ERROR_NOFILE)?;
        let entries = self.list()?;
        for e in &entries {
            if e.name == sfn && !e.is_dir {
                if buf.len() < e.size as usize {
                    return Err(FAT_ERROR_NOSPACE);
                }
                let mut remaining = e.size as usize;
                let mut written = 0usize;
                let mut cluster = e.first_cluster;
                let csize = (self.sectors_per_cluster * 512) as usize;
                let mut cbuf = alloc::vec![0u8; csize];
                while remaining > 0 {
                    if self.is_eoc(cluster) || cluster < 2 {
                        return Err(FAT_ERROR_NOFS);
                    }
                    let r = self.read_cluster(cluster, &mut cbuf);
                    if r != 0 {
                        return Err(r);
                    }
                    let take = remaining.min(csize);
                    buf[written..written + take].copy_from_slice(&cbuf[..take]);
                    written += take;
                    remaining -= take;
                    if remaining == 0 {
                        break;
                    }
                    cluster = self.fat_next(cluster)?;
                }
                return Ok(e.size);
            }
        }
        Err(FAT_ERROR_NOFILE)
    }
}
