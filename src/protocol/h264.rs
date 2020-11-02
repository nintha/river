pub struct Nalu {
    data: Vec<u8>,
}

impl Nalu {
    pub fn new(bytes: Vec<u8>) -> Self {
        Nalu { data: bytes }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    pub fn get_nal_ref_idc(&self) -> u8 {
        self.data[0] >> 5
    }

    pub fn get_nal_unit_type(&self) -> u8 {
        self.data[0] & 0x1F
    }

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
}

