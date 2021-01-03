use crate::protocol::rtmp::{RtmpMessage, ChunkMessageType};

/// # AAC rtmp头部信息封装
/// AAC音频文件的每一帧都由一个ADTS头和AAC ES(AAC音频数据)组成。
///
/// 以下是AAC音频数据格式
/// 第一个byte包含音频的编码参数：
/// ```
/// 1-4bit: audioCodeId
/// 5-6bit: 采样率 00 5.5KHZ, 01 11KHZ, 10 22KHZ, 11 44KHZ
/// 7 bit: 采样长度 0 8bit, 1 16bit
/// 8 bit: 立体声 0 单声道, 1 双声道
/// ```
/// ## 第一帧 AAC sequence header
/// 第一帧共4个byte：
/// 1. 第1个byte ： audioCodeId=10，如果是44KHZ、16bit、双声道，
/// 第一个byte是0xAF。如果实际采样率不是5.5KHZ、11KHZ、22KHZ、44KHZ，
/// 就选一个接近的。
/// 2. 第2个byte ： 0x00 表示是sequence header
/// 3. 第3-4个byte ： 0x14,0x10
///
/// 其他帧 AAC raw data
/// 1) 第1个byte ： audioCodeId=10，如果是44KHZ、16bit、双声道，
/// 第一个byte是0xAF。如果实际采样率不是5.5KHZ、11KHZ、22KHZ、44KHZ，
/// 就选一个接近的。
/// 2) 第2个byte ： 0x01 表示是raw data
/// 3) 第3byte开始 ： 去掉前7个byte的AAC头之后的AAC数据。
pub struct AAC {
    inner: Vec<u8>,
}

#[allow(unused)]
impl AAC {
    pub fn from_rtmp_message(msg: &RtmpMessage, header: &RtmpMessage) -> Option<Self> {
        if msg.header.message_type != ChunkMessageType::AudioMessage {
            return None;
        }

        let adts_header = AAC::gen_adts_header(msg.body.len() as u16);

        let mut bytes = vec![];
        bytes.extend_from_slice(&adts_header);
        bytes.extend_from_slice(&msg.body);
        Some(Self {
            inner: bytes,
        })
    }

    fn gen_adts_header(len: u16) -> [u8; 7] {
        let len = len + 7;
        let mut header = [0xFF, 0xF1, 0x50, 0x20, 0x00, 0x3F, 0xFC];
        let byte3 = 0b0000_0011u8 & ((len >> 12) as u8);
        header[3] |= byte3;
        let byte4 = (len << 5 >> 3) as u8;
        header[4] |= byte4;
        let byte5 = (len << 5) as u8;
        header[5] |= byte5;
        header
    }

    pub fn is_sequence_header(&self) -> bool {
        self.inner[1] == 0x00
    }

    pub fn is_raw_data(&self) -> bool {
        self.inner[1] != 0x00
    }
}

impl AsRef<[u8]> for AAC {
    fn as_ref(&self) -> &[u8] {
        &self.inner
    }
}

pub struct ADTS {
    // 1 bit; 0: MPEG-4, 1: MPEG-2
    pub id: bool,
    // 1 bit; 0: CRC, 1: no-CRC
    pub protection_absent: bool,
    /// 2 bits; the MPEG-4 Audio Object Type minus 1
    pub profile: u8,
    /// 4 bits; 15 is forbidden
    pub sampling_frequency_index: u8,
    /// set to 0 when encoding, ignore when decoding
    pub private_bit: bool,
    /// 3 bits;
    pub channel_configuration: u8,
    pub originality: bool,
    pub home: bool,
    pub copyright_identification_bit: bool,
    pub copyright_identification_start: bool,
    // 13 bits;
    pub aac_frame_lenght: u16,
    // 11 bits;
    pub adts_buffer_fullness: u16,
    // 2 bits;
    pub no_raw_data_blocks_in_frame: u8,
    /// 16 bits;
    pub crc: u16,
}

impl ADTS {
    /// 12 bits
    pub const SYNC_WORD: u16 = 0xFFF;
    /// 2 bits
    pub const LAYER: u8 = 0;

    pub fn to_bytes(&self) -> Vec<u8> {
        unimplemented!()
    }
}