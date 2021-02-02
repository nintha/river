use byteorder::{BigEndian, ByteOrder};

use crate::protocol::rtmp::{RtmpContext, RtmpMessage, ChunkMessageType};

/// H264编码数据存储或传输的基本单元
pub struct Nalu {
    inner: Vec<u8>,
    pub is_key_frame: bool,
}

impl Nalu {
    pub const UNIT_TYPE_SPS: u8 = 7;
    pub const UNIT_TYPE_PPS: u8 = 8;

    /// RtmpMessage to Nalus
    pub fn from_rtmp_message(msg: &RtmpMessage) -> Vec<Nalu> {
        if msg.header.message_type != ChunkMessageType::VideoMessage {
            return vec![];
        }

        let bytes = &msg.body;
        let mut nalus = vec![];

        let frame_type = bytes[0];
        let is_key_frame = frame_type == 0x17;
        let mut read_index = 1;
        let acv_packet_type = bytes[read_index];
        read_index += 4;

        // AVCDecoderConfigurationRecord（AVC sequence header）
        if acv_packet_type == 0 {
            read_index += 5;
            let sps_num = &bytes[read_index] & 0x1F;
            read_index += 1;
            for _ in 0..sps_num as usize {
                let data_len = BigEndian::read_u16(&bytes[read_index..]);
                read_index += 2;
                let data = &bytes[read_index..(read_index + data_len as usize)];
                read_index += data_len as usize;

                let mut nalu_bytes: Vec<u8> = vec![0x00, 0x00, 0x00, 0x01];
                nalu_bytes.extend_from_slice(data);
                nalus.push(Self { inner: nalu_bytes, is_key_frame });
            }
            let num_of_pps = &bytes[read_index] & 0x1F;
            read_index += 1;
            for _ in 0..num_of_pps as usize {
                let data_len = BigEndian::read_u16(&bytes[read_index..]);
                read_index += 2;
                let data = &bytes[read_index..(read_index + data_len as usize)];
                read_index += data_len as usize;

                let mut nalu_bytes: Vec<u8> = vec![0x00, 0x00, 0x00, 0x01];
                nalu_bytes.extend_from_slice(data);
                nalus.push(Self { inner: nalu_bytes, is_key_frame });
            }
        }
        // One or more NALUs (Full frames are required)
        else if acv_packet_type == 1 {
            loop {
                if read_index >= bytes.len() {
                    break;
                }
                let data_len = BigEndian::read_u32(&bytes[read_index..]);
                read_index += 4;
                let data = &bytes[read_index..(read_index + data_len as usize)];
                read_index += data_len as usize;

                let mut nalu_bytes: Vec<u8> = vec![0x00, 0x00, 0x00, 0x01];
                nalu_bytes.extend_from_slice(data);
                nalus.push(Self { inner: nalu_bytes, is_key_frame });
            }
        } else {
            log::warn!("unknown acv packet type");
        };
        nalus
    }

    /// 帧优先级
    #[allow(unused)]
    pub fn get_nal_ref_idc(&self) -> u8 {
        self.inner[0] >> 5
    }

    /// 帧类型
    #[allow(unused)]
    pub fn get_nal_unit_type(&self) -> u8 {
        self.inner[4] & 0x1F
    }

    #[allow(unused)]
    pub fn nalu_type_desc(&self) -> String {
        let priority: String = match self.get_nal_ref_idc() {
            0 => "DISPOSABLE".into(),
            1 => "LOW".into(),
            2 => "HIGH".into(),
            3 => "HIGHEST".into(),
            _ => "UNKNOWN".into(),
        };

        let t: String = match self.get_nal_unit_type() {
            1 => "SLICE".into(),
            2 => "DPA".into(),
            3 => "DPB".into(),
            4 => "DPC".into(),
            5 => "IDR".into(),
            6 => "SEI".into(),
            7 => "SPS".into(),
            8 => "PPS".into(),
            9 => "AUD".into(),
            10 => "EOSEQ".into(),
            11 => "EOSTREAM".into(),
            12 => "FILL".into(),
            _ => "UNKNOWN".into(),
        };

        format!("{}::{}", priority, t)
    }

    pub fn to_avcc_format(&self) -> Vec<u8> {
        let origin = self.as_ref();
        let mut bytes = vec![0x00, 0x00, 0x00, 0x00];
        for i in 4..origin.len() {
            // remove prevention byte
            // if origin[i - 2] == 0 && origin[i - 1] == 0 && origin[i] == 3 {
            //    if i < origin.len() && [0u8, 1, 2, 3].contains(&origin[i + 1]) {
            //        continue;
            //    }
            // }

            bytes.push(origin[i]);
        }
        let len = (bytes.len() - 4) as u32;
        bytes[0] = (len >> 24) as u8;
        bytes[1] = (len >> 16) as u8;
        bytes[2] = (len >> 8) as u8;
        bytes[3] = len as u8;
        bytes
    }
}

impl AsRef<[u8]> for Nalu {
    fn as_ref(&self) -> &[u8] {
        self.inner.as_ref()
    }
}

/// # VideoTagHeader
///
/// ## Frame Type
///
/// Type: UB [4]
///
/// Type of video frame. The following values are defined:
/// 1 = key frame (for AVC, a seekable frame)
/// 2 = inter frame (for AVC, a non-seekable frame)
/// 3 = disposable inter frame (H.263 only)
/// 4 = generated key frame (reserved for server use only)
/// 5 = video info/command frame
///
/// ## CodecID
///
/// Type: UB [4]
///
/// Codec Identifier. The following values are defined:
/// 2 = Sorenson H.263
/// 3 = Screen video
/// 4 = On2 VP6
/// 5 = On2 VP6 with alpha channel
/// 6 = Screen video version 2
/// 7 = AVC
///
/// ## AVCPacketType
///
/// Type: IF CodecID == 7, UI8
///
/// The following values are defined:
/// 0 = AVC sequence header
/// 1 = AVC NALU
/// 2 = AVC end of sequence (lower level NALU sequence ender is not required or supported)
///
/// ## CompositionTime
///
/// Type: IF CodecID == 7, SI24
///
/// IF AVCPacketType == 1
///     Composition time offset
/// ELSE
///     0
/// See ISO 14496-12, 8.15.3 for an explanation of composition
/// times. The offset in an FLV file is always in milliseconds.
#[allow(unused)]
pub fn handle_video_data(bytes: &[u8], ctx: &RtmpContext) {
    let frame_type = bytes[0];
    let mut read_index = 1;
    let acv_packet_type = bytes[read_index];
    read_index += 1;

    // AVC时，全0，无意义（作业时间）
    let _composition_time_offset = &bytes[read_index..read_index + 3];
    read_index += 3;

    log::debug!(
        "[peer={}] video frame type = {:#04X}, acv_packet_type={:#04X}",
        &ctx.peer_addr,
        frame_type,
        acv_packet_type
    );

    // AVCDecoderConfigurationRecord（AVC sequence header）
    if acv_packet_type == 0 {
        // let config_version = &bytes[read_index];
        // let avc_profile_indication = &bytes[read_index + 1];
        // let profile_compatibility = &bytes[read_index + 2];
        // let avc_level_indication = &bytes[read_index + 3];
        // let length_size_minus_one = &bytes[read_index + 4];
        read_index += 5;
        let sps_num = &bytes[read_index] & 0x1F;
        read_index += 1;
        // println!("sps_num={}", sps_num);
        // println!("config_version={:#04X}", config_version);
        // println!("avc_profile_indication={:#04X}", avc_profile_indication);
        // println!("profile_compatibility={:#04X}", profile_compatibility);
        // println!("avc_level_indication={:#04X}", avc_level_indication);
        // println!("length_size_minus_one={:#04X}", length_size_minus_one);
        for _ in 0..sps_num as usize {
            let data_len = BigEndian::read_u16(&bytes[read_index..]);
            read_index += 2;
            let data = &bytes[read_index..(read_index + data_len as usize)];
            read_index += data_len as usize;
            // println!("len={}, sps data:\n{}", data_len, bytes_hex_format(data));

            let mut nalu_bytes: Vec<u8> = vec![0x00, 0x00, 0x00, 0x01];
            nalu_bytes.extend_from_slice(data);
            handle_nalu(nalu_bytes);
        }
        let num_of_pps = &bytes[read_index] & 0x1F;
        read_index += 1;
        // println!("pps num = {}", num_of_pps);
        for _ in 0..num_of_pps as usize {
            let data_len = BigEndian::read_u16(&bytes[read_index..]);
            read_index += 2;
            let data = &bytes[read_index..(read_index + data_len as usize)];
            read_index += data_len as usize;
            // println!("len={}, pps data:\n{}", data_len, bytes_hex_format(data));

            let mut nalu_bytes: Vec<u8> = vec![0x00, 0x00, 0x00, 0x01];
            nalu_bytes.extend_from_slice(data);
            handle_nalu(nalu_bytes);
        }
    }
    // One or more NALUs (Full frames are required)
    else if acv_packet_type == 1 {
        loop {
            if read_index >= bytes.len() {
                break;
            }
            let data_len = BigEndian::read_u32(&bytes[read_index..]);
            read_index += 4;
            let data = &bytes[read_index..(read_index + data_len as usize)];
            read_index += data_len as usize;
            // println!("NALU Type: {}, len={}", nalu_type_desc(&data[0]), data_len);
            // println!("len={}, nalu data:\n{}", data_len, bytes_hex_format(data));

            let mut nalu_bytes: Vec<u8> = vec![0x00, 0x00, 0x00, 0x01];
            nalu_bytes.extend_from_slice(data);
            handle_nalu(nalu_bytes);
        }
    } else {
        unreachable!("unknown acv packet type")
    };

    fn handle_nalu(nalu_bytes: Vec<u8>) {}
}
