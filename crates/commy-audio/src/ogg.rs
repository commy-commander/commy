use crate::SAMPLE_RATE;

pub const OPUS_PRE_SKIP: u16 = 312;
const CAPTURE_PATTERN: &[u8; 4] = b"OggS";
const BEGINNING_OF_STREAM: u8 = 0x02;
const END_OF_STREAM: u8 = 0x04;
const MAX_SEGMENTS_PER_PAGE: usize = 255;
const PACKETS_PER_PAGE: usize = 50;
const OGG_CRC_POLYNOMIAL: u32 = 0x04c1_1db7;
const VENDOR: &[u8] = b"commy";

pub fn ogg_crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &byte in bytes {
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ OGG_CRC_POLYNOMIAL } else { crc << 1 };
        }
    }
    crc
}

pub struct OggOpusWriter {
    out: Vec<u8>,
    serial: u32,
    sequence: u32,
    granule: u64,
    lacing: Vec<u8>,
    data: Vec<u8>,
    packets_on_page: usize,
}

impl OggOpusWriter {
    pub fn new(serial: u32) -> Self {
        let mut writer = Self {
            out: Vec::new(),
            serial,
            sequence: 0,
            granule: 0,
            lacing: Vec::new(),
            data: Vec::new(),
            packets_on_page: 0,
        };
        writer.write_page(&[&opus_head()], 0, BEGINNING_OF_STREAM);
        writer.write_page(&[&opus_tags()], 0, 0);
        writer
    }

    pub fn push(&mut self, packet: &[u8], samples: u32) {
        let segments = packet.len() / 255 + 1;
        if self.lacing.len() + segments > MAX_SEGMENTS_PER_PAGE {
            self.flush(0);
        }
        let mut rest = packet.len();
        while rest >= 255 {
            self.lacing.push(255);
            rest -= 255;
        }
        self.lacing.push(rest as u8);
        self.data.extend_from_slice(packet);
        self.granule += u64::from(samples);
        self.packets_on_page += 1;
        if self.packets_on_page >= PACKETS_PER_PAGE {
            self.flush(0);
        }
    }

    pub fn duration_ms(&self) -> u64 {
        self.granule.saturating_sub(u64::from(OPUS_PRE_SKIP)) * 1000 / u64::from(SAMPLE_RATE)
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.flush(END_OF_STREAM);
        self.out
    }

    fn flush(&mut self, flags: u8) {
        if self.lacing.is_empty() && flags != END_OF_STREAM {
            return;
        }
        let lacing = std::mem::take(&mut self.lacing);
        let data = std::mem::take(&mut self.data);
        self.packets_on_page = 0;
        self.write_raw_page(&lacing, &data, self.granule, flags);
    }

    fn write_page(&mut self, packets: &[&[u8]], granule: u64, flags: u8) {
        let mut lacing = Vec::new();
        let mut data = Vec::new();
        for packet in packets {
            let mut rest = packet.len();
            while rest >= 255 {
                lacing.push(255);
                rest -= 255;
            }
            lacing.push(rest as u8);
            data.extend_from_slice(packet);
        }
        self.write_raw_page(&lacing, &data, granule, flags);
    }

    fn write_raw_page(&mut self, lacing: &[u8], data: &[u8], granule: u64, flags: u8) {
        let start = self.out.len();
        self.out.extend_from_slice(CAPTURE_PATTERN);
        self.out.push(0);
        self.out.push(flags);
        self.out.extend_from_slice(&granule.to_le_bytes());
        self.out.extend_from_slice(&self.serial.to_le_bytes());
        self.out.extend_from_slice(&self.sequence.to_le_bytes());
        self.out.extend_from_slice(&[0, 0, 0, 0]);
        self.out.push(lacing.len() as u8);
        self.out.extend_from_slice(lacing);
        self.out.extend_from_slice(data);
        let crc = ogg_crc32(&self.out[start..]);
        self.out[start + 22..start + 26].copy_from_slice(&crc.to_le_bytes());
        self.sequence += 1;
    }
}

fn opus_head() -> Vec<u8> {
    let mut head = Vec::with_capacity(19);
    head.extend_from_slice(b"OpusHead");
    head.push(1);
    head.push(1);
    head.extend_from_slice(&OPUS_PRE_SKIP.to_le_bytes());
    head.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    head.extend_from_slice(&0u16.to_le_bytes());
    head.push(0);
    head
}

fn opus_tags() -> Vec<u8> {
    let mut tags = Vec::new();
    tags.extend_from_slice(b"OpusTags");
    tags.extend_from_slice(&(VENDOR.len() as u32).to_le_bytes());
    tags.extend_from_slice(VENDOR);
    tags.extend_from_slice(&0u32.to_le_bytes());
    tags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FRAME_SIZE;

    struct Page<'a> {
        flags: u8,
        granule: u64,
        serial: u32,
        sequence: u32,
        packets: Vec<&'a [u8]>,
    }

    fn parse(bytes: &[u8]) -> Vec<Page<'_>> {
        let mut pages = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            assert_eq!(&bytes[at..at + 4], b"OggS", "capture pattern at {at}");
            assert_eq!(bytes[at + 4], 0);
            let flags = bytes[at + 5];
            let granule = u64::from_le_bytes(bytes[at + 6..at + 14].try_into().unwrap());
            let serial = u32::from_le_bytes(bytes[at + 14..at + 18].try_into().unwrap());
            let sequence = u32::from_le_bytes(bytes[at + 18..at + 22].try_into().unwrap());
            let crc = u32::from_le_bytes(bytes[at + 22..at + 26].try_into().unwrap());
            let segments = bytes[at + 26] as usize;
            let lacing = &bytes[at + 27..at + 27 + segments];
            let body_len: usize = lacing.iter().map(|&l| l as usize).sum();
            let end = at + 27 + segments + body_len;
            let mut zeroed = bytes[at..end].to_vec();
            zeroed[22..26].copy_from_slice(&[0, 0, 0, 0]);
            assert_eq!(ogg_crc32(&zeroed), crc, "page {sequence} checksum");
            let mut packets = Vec::new();
            let mut pos = at + 27 + segments;
            let mut len = 0;
            for &l in lacing {
                len += l as usize;
                if l < 255 {
                    packets.push(&bytes[pos..pos + len]);
                    pos += len;
                    len = 0;
                }
            }
            assert_eq!(len, 0, "no packet continues onto the next page");
            pages.push(Page { flags, granule, serial, sequence, packets });
            at = end;
        }
        pages
    }

    #[test]
    fn crc_matches_the_ogg_polynomial() {
        assert_eq!(ogg_crc32(b""), 0);
        assert_eq!(ogg_crc32(b"123456789"), 0x89a1_897f);
        assert_eq!(ogg_crc32(b"OggS"), 0x5fb0_a94f);
    }

    #[test]
    fn an_empty_recording_is_still_a_valid_stream() {
        let bytes = OggOpusWriter::new(7).finish();
        let pages = parse(&bytes);
        assert_eq!(pages.len(), 3);
        assert_eq!(pages[0].flags, BEGINNING_OF_STREAM);
        assert_eq!(pages[0].packets.len(), 1);
        assert_eq!(&pages[0].packets[0][..8], b"OpusHead");
        assert_eq!(pages[0].packets[0][9], 1, "mono");
        assert_eq!(u16::from_le_bytes([pages[0].packets[0][10], pages[0].packets[0][11]]), OPUS_PRE_SKIP);
        assert_eq!(u32::from_le_bytes(pages[0].packets[0][12..16].try_into().unwrap()), SAMPLE_RATE);
        assert_eq!(&pages[1].packets[0][..8], b"OpusTags");
        assert_eq!(pages[2].flags, END_OF_STREAM);
        assert!(pages[2].packets.is_empty());
        assert!(pages.iter().all(|p| p.serial == 7));
        assert_eq!(pages.iter().map(|p| p.sequence).collect::<Vec<_>>(), vec![0, 1, 2]);
    }

    #[test]
    fn packets_are_paged_with_running_granule_positions() {
        let mut writer = OggOpusWriter::new(1);
        for i in 0..120u32 {
            let packet = vec![i as u8; 40 + (i % 7) as usize];
            writer.push(&packet, FRAME_SIZE as u32);
        }
        assert_eq!(writer.duration_ms(), (120 * FRAME_SIZE as u64 - u64::from(OPUS_PRE_SKIP)) * 1000 / 48_000);
        let bytes = writer.finish();
        let pages = parse(&bytes);
        let audio: Vec<&Page> = pages.iter().skip(2).collect();
        assert_eq!(audio.iter().map(|p| p.packets.len()).sum::<usize>(), 120);
        assert_eq!(audio[0].packets.len(), PACKETS_PER_PAGE);
        assert_eq!(audio[0].granule, (PACKETS_PER_PAGE * FRAME_SIZE) as u64);
        assert_eq!(audio.last().unwrap().granule, (120 * FRAME_SIZE) as u64);
        assert_eq!(audio.last().unwrap().flags, END_OF_STREAM);
        assert_eq!(audio[0].packets[3], &vec![3u8; 43][..]);
        assert!(pages.iter().take(pages.len() - 1).all(|p| p.flags != END_OF_STREAM));
    }

    #[test]
    fn long_packets_use_several_lacing_values_and_never_overflow_a_page() {
        let mut writer = OggOpusWriter::new(1);
        let big = vec![9u8; 1275];
        for _ in 0..60 {
            writer.push(&big, 960);
        }
        let bytes = writer.finish();
        let pages = parse(&bytes);
        let packets: Vec<&[u8]> = pages.iter().skip(2).flat_map(|p| p.packets.iter().copied()).collect();
        assert_eq!(packets.len(), 60);
        assert!(packets.iter().all(|p| p.len() == 1275));
        let exact = vec![1u8; 255];
        let mut writer = OggOpusWriter::new(1);
        writer.push(&exact, 960);
        writer.push(&[], 960);
        let bytes = writer.finish();
        let pages = parse(&bytes);
        assert_eq!(pages[2].packets.len(), 2);
        assert_eq!(pages[2].packets[0].len(), 255);
        assert!(pages[2].packets[1].is_empty());
    }
}
