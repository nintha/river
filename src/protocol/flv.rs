use byteorder::{BigEndian, ByteOrder};

use crate::protocol::rtmp::{ChunkMessageType, RtmpMessage};
use crate::util::spawn_and_log_error;
use smol::channel::Receiver;
use std::convert::TryFrom;

use crate::rtmp_server::eventbus_map;
use chrono::Local;
use smol::io::AsyncWriteExt;
use std::time::{Duration, Instant};

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

impl TryFrom<RtmpMessage> for FlvTag {
    type Error = anyhow::Error;

    /// 当RtmpMessage类型不是音频或视频的时候，会返回Error
    fn try_from(mut msg: RtmpMessage) -> Result<Self, Self::Error> {
        let mut raw_data = vec![];
        // data type
        match msg.header.message_type {
            ChunkMessageType::AudioMessage => raw_data.push(0x08),
            ChunkMessageType::VideoMessage => raw_data.push(0x09),
            _ => Err(anyhow::anyhow!(
                "[FlvTag] invalid message type, {:?}",
                msg.header.message_type
            ))?,
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
}

impl AsRef<[u8]> for FlvTag {
    fn as_ref(&self) -> &[u8] {
        self.raw_data.as_ref()
    }
}

/// 后台保存FLV文件
#[allow(unused)]
pub fn save_flv_background(stream_name: &str, peer_addr: String) {
    if let Some(eventbus) = eventbus_map().get(stream_name) {
        let flv_rx = eventbus.register_receiver();
        spawn_and_log_error(handle_flv_rx(flv_rx, stream_name.to_owned(), peer_addr));
    }
}

/// Rtmp流输出到FLV文件
async fn handle_flv_rx(
    flv_rx: Receiver<RtmpMessage>,
    stream_name: String,
    peer_addr: String,
) -> anyhow::Result<()> {
    let tmp_dir = "tmp";
    if smol::fs::read_dir(tmp_dir).await.is_err() {
        smol::fs::create_dir_all(tmp_dir).await?;
    }

    let mut file = smol::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("tmp/output.flv")
        .await?;

    // write header
    file.write_all(&FLV_HEADER_WITH_TAG0).await?;

    let ctx_begin_timestamp = Local::now().timestamp_millis();
    let mut last_flush_time = Instant::now();
    let min_flush_duration = Duration::from_secs(2);
    while let Ok(mut msg) = flv_rx.recv().await {
        msg.header.timestamp = (Local::now().timestamp_millis() - ctx_begin_timestamp) as u32;
        let flv_tag = FlvTag::try_from(msg)?;
        file.write_all(flv_tag.as_ref()).await?;
        file.write_all(&(flv_tag.as_ref().len() as u32).to_be_bytes())
            .await?;

        if last_flush_time.elapsed() > min_flush_duration {
            last_flush_time = Instant::now();
            file.flush().await?
        }
    }
    log::warn!(
        "[peer={}][handle_flv_rx] closed, stream_name={}",
        peer_addr,
        stream_name
    );
    Ok(())
}
