use std::{u32, vec};
use crate::rtmp_server::{eventbus_map, meta_data_map, video_header_map};
use crate::util::spawn_and_log_error;
use smol::channel::Receiver;
use crate::protocol::rtmp::RtmpMessage;
use crate::protocol::h264::Nalu;
use smol::io::AsyncWriteExt;

/// fps = timescale / duration
#[derive(Clone)]
pub struct Track {
    pub id: u32,
    pub duration: u32,
    pub timescale: u32,
    pub width: u16,
    pub height: u16,
    pub volume: u16,
    pub dts: u32,
    pub pps_list: Vec<Vec<u8>>,
    pub sps_list: Vec<Vec<u8>>,
}

impl Track {
    pub const DEFAULT_TIMESCALE: u32 = 1000_000;
    pub const DEFAULT_ID: u32 = 1;
}

impl Default for Track {
    fn default() -> Self {
        Self {
            id: Track::DEFAULT_ID,
            duration: 0,
            timescale: Track::DEFAULT_TIMESCALE,
            width: 0,
            height: 0,
            volume: 0,
            dts: 0,
            pps_list: vec![],
            sps_list: vec![],
        }
    }
}


#[derive(Clone)]
pub struct Sample {
    pub size: u32,
    pub duration: u32,
    pub cts: u32,
    pub flags: Flags,
}

impl Sample {
    pub fn new(size: u32, duration: u32, cts: u32, key_frame: bool) -> Self {
        Self {
            size,
            duration,
            cts,
            flags: Flags {
                is_leading: 0,
                is_depended_on: 0,
                has_redundancy: 0,
                depands_on: if key_frame { 2 } else { 1 },
                padding_value: 0,
                is_non_sync: if key_frame { 0 } else { 1 },
                degrad_prio: 0,
            },
        }
    }
}

#[derive(Clone)]
pub struct Flags {
    pub is_leading: u8,
    pub is_depended_on: u8,
    pub has_redundancy: u8,
    pub depands_on: u8,
    pub padding_value: u8,
    pub is_non_sync: u8,
    pub degrad_prio: u16,
}

impl Flags {
    pub fn as_byte(&self) -> u8 {
        self.depands_on << 4 | self.is_depended_on << 2 | self.has_redundancy as u8
    }

    /// in trun box
    pub fn as_four_byte(&self) -> [u8; 4] {
        [
            self.is_leading << 2 | self.depands_on,
            self.is_depended_on << 6 | self.has_redundancy << 6 | self.padding_value << 1 | self.is_non_sync,
            (self.degrad_prio >> 8) as u8,
            self.degrad_prio as u8,
        ]
    }
}

pub struct Fmp4Encoder {
    track: Track,
    sn: u32,
}

impl Fmp4Encoder {
    pub fn new(track: Track) -> Self {
        Self {
            track,
            sn: 0,
        }
    }

    pub fn init_segment(&self) -> Vec<u8> {
        let mut ftyp = ftyp();
        let mut movie = moov(&vec![self.track.clone()], Track::DEFAULT_TIMESCALE, self.track.timescale);
        let total_len = ftyp.len() + movie.len();

        let mut buffer = Vec::with_capacity(total_len);
        buffer.append(&mut ftyp);
        buffer.append(&mut movie);

        buffer
    }

    pub fn wrap_frame(&mut self, data: &[u8], key_frame: bool) -> Vec<u8> {
        let sample = Sample::new(
            data.len() as u32,
            self.track.duration,
            0,
            key_frame,
        );

        let mut buffer = moof(self.sn, self.track.dts, &self.track, &vec![sample]);
        buffer.append(&mut mdat(data));

        self.track.dts += self.track.duration;
        self.sn += 1;

        buffer
    }
}

fn moof(sn: u32, base_media_decode_time: u32, track: &Track, samples: &[Sample]) -> Vec<u8> {
    mp4_box(b"moof", vec![&mfhd(sn), &traf(track, base_media_decode_time, samples)])
}

/// movie data
fn mdat(data: &[u8]) -> Vec<u8> {
    mp4_box(b"mdat", vec![data])
}

fn mp4_box(box_type: &[u8; 4], payloads: Vec<&[u8]>) -> Vec<u8> {
    let size: u32 = 8 + payloads.iter().map(|x| x.len() as u32).sum::<u32>();
    let mut buffer = Vec::with_capacity(size as usize);
    buffer.extend_from_slice(&size.to_be_bytes());
    buffer.extend_from_slice(box_type);

    for p in payloads {
        buffer.extend_from_slice(p);
    }

    buffer
}

fn mfhd(sn: u32) -> Vec<u8> {
    let bytes: [u8; 8] = [
        0x00,
        0x00, 0x00, 0x00, // flags
        (sn >> 24) as u8,
        (sn >> 16) as u8,
        (sn >> 8) as u8,
        sn as u8, // sequence_number
    ];
    mp4_box(b"mfhd", vec![&bytes])
}

fn traf(track: &Track, base_media_decode_time: u32, samples: &[Sample]) -> Vec<u8> {
    let sample_dependency_table = sdtp(samples);
    let id = track.id;

    let tfhd = {
        let bytes: [u8; 8] = [
            0x00, // version 0
            0x00, 0x00, 0x00, // flags
            (id >> 24) as u8,
            (id >> 16) as u8,
            (id >> 8) as u8,
            (id as u8), // track_ID
        ];
        mp4_box(b"tfhd", vec![&bytes])
    };

    let tfdt = {
        let bytes: [u8; 8] = [
            0x00, // version 0
            0x00, 0x00, 0x00, // flags
            (base_media_decode_time >> 24) as u8,
            (base_media_decode_time >> 16) as u8,
            (base_media_decode_time >> 8) as u8,
            (base_media_decode_time as u8), // baseMediaDecodeTime
        ];
        mp4_box(b"tfdt", vec![&bytes])
    };

    let trun = trun(track, sample_dependency_table.len() as u32 +
        16 + // tfhd
        16 + // tfdt
        8 +  // traf header
        16 + // mfhd
        8 +  // moof header
        8, samples);

    mp4_box(b"traf", vec![&tfhd, &tfdt, &trun, &sample_dependency_table])
}

fn trun(_track: &Track, offset: u32, samples: &[Sample]) -> Vec<u8> {
    let sample_count = samples.len() as u32;
    let data_offset = offset + 8 + 12 + 16 * sample_count;

    let mut buffer = vec![];
    buffer.push(0x00); // version 0
    buffer.extend_from_slice(&[0x00, 0x0F, 0x01]); // flags
    buffer.extend_from_slice(&sample_count.to_be_bytes());
    buffer.extend_from_slice(&data_offset.to_be_bytes());

    for s in samples {
        buffer.extend_from_slice(&s.duration.to_be_bytes());
        buffer.extend_from_slice(&s.size.to_be_bytes());
        buffer.extend_from_slice(&s.flags.as_four_byte());
        buffer.extend_from_slice(&s.cts.to_be_bytes());
    }

    mp4_box(b"trun", vec![&buffer])
}

fn sdtp(samples: &[Sample]) -> Vec<u8> {
    let mut buffer = Vec::with_capacity(samples.len() + 4);
    // leave the full box header (4 bytes) all zero
    buffer.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, ]);

    for s in samples {
        buffer.push(s.flags.as_byte());
    }
    mp4_box(b"sdtp", vec![&buffer])
}

/// file type
fn ftyp() -> Vec<u8> {
    const MAJOR_BRAND: [u8; 4] = *b"isom";
    const MINOR_VERSION: [u8; 4] = [0, 0, 0, 1];
    const AVC_BRAND: [u8; 4] = *b"avc1";

    mp4_box(b"ftyp", vec![&MAJOR_BRAND, &MINOR_VERSION, &MAJOR_BRAND, &AVC_BRAND])
}

fn mvhd(timescale: u32, duration: u32) -> Vec<u8> {
    let bytes = vec![
        0x00, // version 0
        0x00, 0x00, 0x00, // flags
        0x00, 0x00, 0x00, 0x01, // creation_time
        0x00, 0x00, 0x00, 0x02, // modification_time
        (timescale >> 24) as u8,
        (timescale >> 16) as u8,
        (timescale >> 8) as u8,
        timescale as u8, // timescale
        (duration >> 24) as u8,
        (duration >> 16) as u8,
        (duration >> 8) as u8,
        duration as u8, // duration
        0x00, 0x01, 0x00, 0x00, // 1.0 rate
        0x01, 0x00, // 1.0 volume
        0x00, 0x00, // reserved
        0x00, 0x00, 0x00, 0x00, // reserved
        0x00, 0x00, 0x00, 0x00, // reserved
        0x00, 0x01, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x01, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x40, 0x00, 0x00, 0x00, // transformation: unity matrix
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, // pre_defined
        0xff, 0xff, 0xff, 0xff, // next_track_ID
    ];

    mp4_box(b"mvhd", vec![&bytes])
}

fn trak(track: &Track) -> Vec<u8> {
    mp4_box(b"trak", vec![&tkhd(&track), &mdia(&track)])
}

fn tkhd(track: &Track) -> Vec<u8> {
    let bytes = vec![
        0x00, // version 0
        0x00, 0x00, 0x07, // flags
        0x00, 0x00, 0x00, 0x00, // creation_time
        0x00, 0x00, 0x00, 0x00, // modification_time
        (track.id >> 24) as u8,
        (track.id >> 16) as u8,
        (track.id >> 8) as u8,
        track.id as u8, // track_ID
        0x00, 0x00, 0x00, 0x00, // reserved
        (track.duration >> 24) as u8,
        (track.duration >> 16) as u8,
        (track.duration >> 8) as u8,
        track.duration as u8, // duration
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, // reserved
        0x00, 0x00, // layer
        0x00, 0x00, // alternate_group
        (track.volume >> 0) as u8, (((track.volume % 1) * 10) >> 0) as u8, // track volume
        0x00, 0x00, // reserved
        0x00, 0x01, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x01, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x40, 0x00, 0x00, 0x00, // transformation: unity matrix
        (track.width >> 8) as u8,
        track.width as u8,
        0x00, 0x00, // width
        (track.height >> 8) as u8,
        track.height as u8,
        0x00, 0x00, // height
    ];
    mp4_box(b"tkhd", vec![&bytes])
}

fn mdia(track: &Track) -> Vec<u8> {
    mp4_box(b"mdia", vec![&mdhd(track.timescale, track.duration), &hdlr(), &minf(track)])
}

fn minf(track: &Track) -> Vec<u8> {
    const VMHD: [u8; 12] = [
        0x00, // version
        0x00, 0x00, 0x01, // flags
        0x00, 0x00, // graphicsmode
        0x00, 0x00,
        0x00, 0x00,
        0x00, 0x00, // opcolor
    ];
    const DREF: [u8; 20] = [
        0x00, // version 0
        0x00, 0x00, 0x00, // flags
        0x00, 0x00, 0x00, 0x01, // entry_count
        0x00, 0x00, 0x00, 0x0c, // entry_size
        0x75, 0x72, 0x6c, 0x20, // 'url' type
        0x00, // version 0
        0x00, 0x00, 0x01, // entry_flags
    ];
    let dinf = mp4_box(b"dinf", vec![&mp4_box(b"dref", vec![&DREF])]);
    mp4_box(b"minf", vec![&mp4_box(b"vmhd", vec![&VMHD]), &dinf, &stbl(&track)])
}

fn mdhd(timescale: u32, duration: u32) -> Vec<u8> {
    let bytes = vec![
        0x00, // version 0
        0x00, 0x00, 0x00, // flags
        0x00, 0x00, 0x00, 0x02, // creation_time
        0x00, 0x00, 0x00, 0x03, // modification_time
        (timescale >> 24) as u8,
        (timescale >> 16) as u8,
        (timescale >> 8) as u8,
        timescale as u8, // timescale
        (duration >> 24) as u8,
        (duration >> 16) as u8,
        (duration >> 8) as u8,
        duration as u8, // duration
        0x55, 0xc4, // 'und' language (undetermined)
        0x00, 0x00,
    ];
    mp4_box(b"mdhd", vec![&bytes])
}

fn hdlr() -> Vec<u8> {
    const VIDEO_HDLR: [u8; 37] = [
        0x00, // version 0
        0x00, 0x00, 0x00, // flags
        0x00, 0x00, 0x00, 0x00, // pre_defined
        0x76, 0x69, 0x64, 0x65, // handler_type: 'vide'
        0x00, 0x00, 0x00, 0x00, // reserved
        0x00, 0x00, 0x00, 0x00, // reserved
        0x00, 0x00, 0x00, 0x00, // reserved
        0x56, 0x69, 0x64, 0x65,
        0x6f, 0x48, 0x61, 0x6e,
        0x64, 0x6c, 0x65, 0x72, 0x00, // name: 'VideoHandler'
    ];
    mp4_box(b"hdlr", vec![&VIDEO_HDLR])
}

fn stbl(track: &Track) -> Vec<u8> {
    const STCO: [u8; 8] = [
        0x00, // version
        0x00, 0x00, 0x00, // flags
        0x00, 0x00, 0x00, 0x00, // entry_count
    ];
    const STTS: [u8; 8] = STCO;
    const STSC: [u8; 8] = STCO;
    const STSZ: [u8; 12] = [
        0x00, // version
        0x00, 0x00, 0x00, // flags
        0x00, 0x00, 0x00, 0x00, // sample_size
        0x00, 0x00, 0x00, 0x00, // sample_count
    ];

    mp4_box(b"stbl", vec![
        &stsd(track),
        &mp4_box(b"stts", vec![&STTS]),
        &mp4_box(b"stsc", vec![&STSC]),
        &mp4_box(b"stsz", vec![&STSZ]),
        &mp4_box(b"stco", vec![&STCO])
    ])
}

fn stsd(track: &Track) -> Vec<u8> {
    const STSD: [u8; 8] = [
        0x00, // version 0
        0x00, 0x00, 0x00, // flags
        0x00, 0x00, 0x00, 0x01
    ];
    mp4_box(b"stsd", vec![&STSD, &avc1(track)])
}

fn avc1(track: &Track) -> Vec<u8> {
    let mut sps = vec![];
    let mut pps = vec![];

    for item in &track.sps_list {
        let length = item.len() as u16;
        sps.extend_from_slice(&length.to_be_bytes());
        sps.extend_from_slice(&item);
    }

    for item in &track.pps_list {
        let length = item.len() as u16;
        pps.extend_from_slice(&length.to_be_bytes());
        pps.extend_from_slice(&item);
    }

    let width = track.width;
    let height = track.height;

    let bytes = vec![
        0x00, 0x00, 0x00, // reserved
        0x00, 0x00, 0x00, // reserved
        0x00, 0x01, // data_reference_index
        0x00, 0x00, // pre_defined
        0x00, 0x00, // reserved
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, // pre_defined
        (width >> 8) as u8,
        width as u8, // width
        (height >> 8) as u8,
        height as u8, // height
        0x00, 0x48, 0x00, 0x00, // horizresolution
        0x00, 0x48, 0x00, 0x00, // vertresolution
        0x00, 0x00, 0x00, 0x00, // reserved
        0x00, 0x01, // frame_count
        0x12,
        0x62, 0x69, 0x6E, 0x65, // binelpro.ru
        0x6C, 0x70, 0x72, 0x6F,
        0x2E, 0x72, 0x75, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, // compressorname
        0x00, 0x18,   // depth = 24
        0xFF, 0xFF
    ];

    mp4_box(b"avc1", vec![&bytes, &avcc(track, &sps, &pps), &btrt()])
}

/// AVCConfigurationBox
fn avcc(track: &Track, sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut bytes = vec![
        0x01, // version
        sps[3], // profile
        sps[4], // profile compat
        sps[5], // level
        0xFC | 3, // lengthSizeMinusOne, hard-coded to 4 bytes
        0xE0 | track.sps_list.len() as u8, // 3bit reserved (111) + numOfSequenceParameterSets
    ];
    bytes.extend_from_slice(sps);
    bytes.push(track.pps_list.len() as u8);
    bytes.extend_from_slice(pps);

    mp4_box(b"avcC", vec![&bytes])
}

fn btrt() -> Vec<u8> {
    const BTRT: [u8; 12] = [
        0x00, 0x1c, 0x9c, 0x80, // bufferSizeDB
        0x00, 0x2d, 0xc6, 0xc0, // maxBitrate
        0x00, 0x2d, 0xc6, 0xc0
    ];
    mp4_box(b"btrt", vec![&BTRT])
}

/// movie extend
fn mvex(tracks: &[Track]) -> Vec<u8> {
    let boxes = tracks.into_iter().map(|t| trex(t)).collect::<Vec<Vec<u8>>>();
    mp4_box(b"mvex", boxes.iter().map(AsRef::as_ref).collect())
}

fn trex(track: &Track) -> Vec<u8> {
    let bytes = [
        0x00, // version 0
        0x00, 0x00, 0x00, // flags
        (track.id >> 24) as u8,
        (track.id >> 16) as u8,
        (track.id >> 8) as u8,
        track.id as u8, // track_ID
        0x00, 0x00, 0x00, 0x01, // default_sample_description_index
        0x00, 0x00, 0x00, 0x00, // default_sample_duration
        0x00, 0x00, 0x00, 0x00, // default_sample_size
        0x00, 0x01, 0x00, 0x01, // default_sample_flags
    ];
    mp4_box(b"trex", vec![&bytes])
}

/// movie box
fn moov(tracks: &[Track], duration: u32, timescale: u32) -> Vec<u8> {
    let boxes = tracks.iter().map(|t| trak(t)).collect::<Vec<Vec<u8>>>();
    let mvhd = mvhd(timescale, duration);
    let mvex = mvex(&tracks);

    let mut payloads: Vec<&[u8]> = vec![];
    payloads.push(&mvhd);
    boxes.iter().for_each(|x| payloads.push(x));
    payloads.push(&mvex);

    mp4_box(b"moov", payloads)
}

/// 后台保存FLV文件
#[allow(unused)]
pub fn save_fmp4_background(stream_name: &str, peer_addr: String) {
    if let Some(eventbus) = eventbus_map().get(stream_name) {
        log::warn!("[peer={}] save_fmp4_background, stream_name={}", peer_addr, stream_name);
        let rx = eventbus.register_receiver();
        spawn_and_log_error(handle_fmp4_rx(rx, stream_name.to_owned(), peer_addr));
    }
}

/// Rtmp流输出到mp4文件
async fn handle_fmp4_rx(
    rx: Receiver<RtmpMessage>,
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
        .open("tmp/output.mp4")
        .await?;

    let meta_data = meta_data_map()
        .get(&stream_name)
        .map(|it| it.value().clone())
        .ok_or_else(|| anyhow::anyhow!(format!("not found meta_data, stream={}", stream_name)))?;

    let video_header = video_header_map()
        .get(&stream_name)
        .map(|it| it.value().clone())
        .ok_or_else(|| anyhow::anyhow!(format!("not found meta_data, stream={}", stream_name)))?;

    let mut sps_list = vec![];
    let mut pps_list = vec![];
    let pioneer_nalus = Nalu::from_rtmp_message(&video_header);
    for nalu in pioneer_nalus {
        let bytes = (&nalu.to_avcc_format()[4..]).to_vec();
        match nalu.get_nal_unit_type() {
            Nalu::UNIT_TYPE_SPS => sps_list.push(bytes),
            Nalu::UNIT_TYPE_PPS => pps_list.push(bytes),
            _ => {}
        }
    }

    log::info!("[peer={}], sps={:?}, pps={:?}", peer_addr, sps_list, pps_list);
    let mut fmp4_encoder = Fmp4Encoder::new(Track {
        duration: (Track::DEFAULT_TIMESCALE as f64 / meta_data.frame_rate) as _,
        timescale: Track::DEFAULT_TIMESCALE,
        width: meta_data.width as _,
        height: meta_data.height as _,
        sps_list,
        pps_list,
        ..Default::default()
    });

    // send video header
    let header = fmp4_encoder.init_segment();
    file.write_all(&header).await?;

    let mut found_key_frame = false;
    while let Ok(msg) = rx.recv().await {
        let nalus = Nalu::from_rtmp_message(&msg);
        for nalu in nalus {
            if !found_key_frame {
                if nalu.is_key_frame {
                    found_key_frame = true;
                } else {
                    continue;
                }
            }

            let bytes = fmp4_encoder.wrap_frame(&nalu.to_avcc_format(), nalu.is_key_frame);
            file.write_all(&bytes).await?;
        }
        file.flush().await?
    }

    log::warn!("[peer={}][handle_fmp4_rx] closed, stream_name={}", peer_addr, stream_name);
    Ok(())
}
