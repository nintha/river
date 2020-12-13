use crate::protocol::rtmp::{RtmpMessage, ChunkMessageType};

/// # AAC rtmp头部信息封装
///
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
///
/// ## G.711U rtmp头部信息封装
/// 1) 第1个byte ： audioCodeId=8，如果是11KHZ、16bit、单声道，
/// 第一个byte是0x86。如果实际采样率不是5.5KHZ、11KHZ、22KHZ、44KHZ，
/// 就选一个接近的。
/// 2) 第2个byte开始是存放G.711U数据。
///
/// ## G.711A rtmp头部信息封装
/// 1) 第1个byte ： audioCodeId=7，如果是11KHZ、16bit、单声道，
/// 第一个byte是0x76。如果实际采样率不是5.5KHZ、11KHZ、22KHZ、44KHZ，
/// 就选一个接近的。
/// 2) 第2个byte开始是存放G.711A数据。
pub struct Aac {
    inner: Vec<u8>,
}

#[allow(unused)]
impl Aac {
    pub fn from_rtmp_message(msg: &RtmpMessage) -> Option<Self> {
        if msg.header.message_type != ChunkMessageType::AudioMessage {
            return None;
        }

        let bytes = &msg.body;
        Some(Self {
            inner: bytes.to_owned(),
        })
    }

    pub fn is_sequence_header(&self) -> bool {
        self.inner[1] == 0x00
    }
}

impl AsRef<[u8]> for Aac {
    fn as_ref(&self) -> &[u8] {
        &self.inner
    }
}