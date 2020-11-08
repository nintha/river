use byteorder::{BigEndian, ByteOrder};

use crate::protocol::rtmp::{ChunkMessageType, RtmpMessage};

pub const FLV_HEADER_WITH_TAG0: [u8; 13] = [
    0x46, 0x4c, 0x56, // signature
    0x01, // version
    0x05, // audio and video flag
    0x00, 0x00, 0x00, 0x09, // header length
    0x00, 0x00, 0x00, 0x00, // tag0 length
];

pub struct FlvTag {
    raw_data: Vec<u8>,
}

#[allow(unused)]
impl FlvTag {
    /// 当RtmpMessage类型不是音频或视频的时候，会返回Error
    pub fn from_rtmp_message(mut msg: RtmpMessage) -> anyhow::Result<Self> {
        let mut raw_data = vec![];
        // data type
        match msg.header.message_type {
            ChunkMessageType::AudioMessage => raw_data.push(0x08),
            ChunkMessageType::VideoMessage => raw_data.push(0x09),
            _ => Err(anyhow::anyhow!("[FlvTag] invalid message type, {:?}", msg.header.message_type))?,
        }
        // data size
        raw_data.extend_from_slice(&(msg.body.len() as u32).to_be_bytes()[1..4]);
        // timestamp
        raw_data.extend_from_slice(&(msg.header.timestamp & 0xFFFFFF).to_be_bytes()[1..4]);
        // timestamp extended
        raw_data.push((msg.header.timestamp >> 24) as u8);
        // stream id
        raw_data.extend_from_slice(&0u32.to_be_bytes()[1..4]);
        // body
        raw_data.append(&mut msg.body);

        Ok(FlvTag { raw_data })
    }

    /// 0x08=audio, 0x09=video, 0x12=script
    pub fn tag_type(&self) -> u8 {
        self.raw_data[0]
    }

    /// 0x08=audio, 0x09=video, 0x12=script
    pub fn data_size(&self) -> u32 {
        BigEndian::read_u24(&self.raw_data[1..4])
    }

    /// ms timestamp
    pub fn timestamp(&self) -> u32 {
        let timestamp_u24 = BigEndian::read_u24(&self.raw_data[4..7]);
        timestamp_u24 | (self.raw_data[7] as u32) << 24
    }

    /// audio/video/script data
    pub fn body(&self) -> &[u8] {
        &self.raw_data[11..]
    }
}

impl AsRef<[u8]> for FlvTag {
    fn as_ref(&self) -> &[u8] {
        self.raw_data.as_ref()
    }
}



