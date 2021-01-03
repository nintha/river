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

        Some(Self {
            inner: msg.body.to_owned(),
        })
    }

    pub fn is_sequence_header(&self) -> bool {
        self.inner[1] == 0x00
    }

    pub fn is_raw_data(&self) -> bool {
        self.inner[1] != 0x00
    }

    /// raw_data -> ADTS
    pub fn to_adts(&self) -> Option<ADTS> {
        if self.is_raw_data() {
            Some(ADTS::with_data(self.inner[2..].to_vec()))
        } else {
            None
        }
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
    /// 2 bits; 0-Main Profile, 1-Low Complexity, 2-Scalable Sampling Rate
    pub profile: u8,
    /// 4 bits; 15 is forbidden
    pub sampling_frequency_index: u8,
    /// set to 0 when encoding, ignore when decoding
    pub private_bit: bool,
    /// 3 bits;
    pub channel_configuration: u8,
    pub copyright_identification_bit: bool,
    pub copyright_identification_start: bool,
    // 13 bits;
    pub aac_frame_length: u16,
    // 2 bits; 表示ADTS帧中有N + 1个AAC原始帧
    pub num_of_raw_data_blocks_in_frame: u8,
    // 音频数据
    pub raw_data: Vec<u8>,
}

impl ADTS {
    pub const HEADER_LEN: u16 = 7;
    /// 12 bits
    pub const SYNC_WORD: u16 = 0xFFF;
    /// 2 bits
    pub const LAYER: u8 = 0;
    /// 1 bit
    pub const PROTECTION_ABSENT: bool = true;
    /// set to 0 when encoding, ignore when decoding,
    pub const ORIGINALITY: bool = false;
    /// set to 0 when encoding, ignore when decoding
    pub const HOME: bool = false;
    /// 11bits; 0x7FF 说明是码率可变的码流
    pub const ADTS_BUFFER_FULLNESS: u16 = 0x7FF;

    /// `len` must be less than or equals to 2^13 - 7
    pub fn with_data(data: Vec<u8>) -> Self {
        if data.len() + ADTS::HEADER_LEN as usize > (2usize << 13) {
            unreachable!("ADTS len must be less than or equals to 2^13 - 7");
        }

        Self {
            id: false,
            profile: 1,
            sampling_frequency_index: 4,
            private_bit: false,
            channel_configuration: 1,
            copyright_identification_bit: false,
            copyright_identification_start: false,
            aac_frame_length: data.len() as u16 + ADTS::HEADER_LEN,
            num_of_raw_data_blocks_in_frame: 0,
            raw_data: data,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut data = vec![0u8; 7];
        data[0] = (ADTS::SYNC_WORD >> 4) as u8;
        data[1] = 0xF0 | bool2u8(self.id) << 3 | ADTS::LAYER << 1 | bool2u8(ADTS::PROTECTION_ABSENT);
        data[2] = self.profile << 6
            | self.sampling_frequency_index << 2
            | bool2u8(self.private_bit) << 1
            | self.channel_configuration >> 2;
        data[3] = self.channel_configuration << 6
            | bool2u8(ADTS::ORIGINALITY) << 5
            | bool2u8(ADTS::HOME) << 4
            | bool2u8(self.copyright_identification_bit) << 3
            | bool2u8(self.copyright_identification_start) << 2
            | (self.aac_frame_length >> 11) as u8;
        data[4] = (self.aac_frame_length >> 3) as u8;
        data[5] = (self.aac_frame_length as u8) << 5 | (ADTS::ADTS_BUFFER_FULLNESS >> 6) as u8;
        data[6] = (ADTS::ADTS_BUFFER_FULLNESS as u8) << 2 | self.num_of_raw_data_blocks_in_frame;

        data.extend_from_slice(&self.raw_data);
        data
    }
}

fn bool2u8(v: bool) -> u8 {
    if v { 0x01 } else { 0x00 }
}