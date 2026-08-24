use crate::h264;
use eiviz_media::EncodedAccessUnit;

pub struct FragmentedMp4 {
    pub bytes: Vec<u8>,
    seq: u32,
    timescale: u32,
    width: u16,
    height: u16,
    avcc: Vec<u8>,
}

impl FragmentedMp4 {
    pub fn new(timescale: u32, width: u16, height: u16, sps: &[u8], pps: &[u8]) -> Self {
        let avcc = avcc_from_sps_pps(sps, pps);
        let mut s = Self {
            bytes: Vec::new(),
            seq: 1,
            timescale,
            width,
            height,
            avcc,
        };
        let mut ftyp_pl = Vec::from(&b"isom"[..]);
        ftyp_pl.extend_from_slice(&0x200u32.to_be_bytes());
        ftyp_pl.extend_from_slice(b"isomiso5avc1mp41");
        s.bytes.extend_from_slice(&bx(b"ftyp", &ftyp_pl));
        s.bytes.extend_from_slice(&s.moov());
        s
    }

    pub fn write_sample(&mut self, au: &EncodedAccessUnit, duration: u32) {
        let mut payload = Vec::new();
        for nal in h264::split_annexb(&au.bytes) {
            if nal.is_empty() {
                continue;
            }
            let t = nal[0] & 0x1f;
            if t == 7 || t == 8 {
                continue;
            }
            payload.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            payload.extend_from_slice(&nal);
        }
        if payload.is_empty() {
            return;
        }
        let flags = if au.keyframe {
            0x02000000u32
        } else {
            0x01000000
        };
        self.bytes
            .extend_from_slice(&self.moof(duration, payload.len() as u32, flags));
        self.bytes.extend_from_slice(&bx(b"mdat", &payload));
        self.seq += 1;
    }

    fn moov(&self) -> Vec<u8> {
        let mut mvhd = Vec::new();
        mvhd.extend_from_slice(&[0u8; 8]);
        mvhd.extend_from_slice(&self.timescale.to_be_bytes());
        mvhd.extend_from_slice(&0u32.to_be_bytes());
        mvhd.extend_from_slice(&0x00010000u32.to_be_bytes());
        mvhd.extend_from_slice(&0x0100u16.to_be_bytes());
        mvhd.extend_from_slice(&[0u8; 10]);
        for n in [0x00010000u32, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000] {
            mvhd.extend_from_slice(&n.to_be_bytes());
        }
        mvhd.extend_from_slice(&[0u8; 24]);
        mvhd.extend_from_slice(&2u32.to_be_bytes());
        let mut inner = full(b"mvhd", 0, 0, &mvhd);
        inner.extend_from_slice(&bx(b"trak", &self.trak()));
        let mut trex = 1u32.to_be_bytes().to_vec();
        trex.extend_from_slice(&1u32.to_be_bytes());
        trex.extend_from_slice(&1001u32.to_be_bytes());
        trex.extend_from_slice(&0u32.to_be_bytes());
        trex.extend_from_slice(&0u32.to_be_bytes());
        inner.extend_from_slice(&bx(b"mvex", &full(b"trex", 0, 0, &trex)));
        bx(b"moov", &inner)
    }

    fn trak(&self) -> Vec<u8> {
        let mut tkhd = Vec::new();
        tkhd.extend_from_slice(&[0u8; 8]);
        tkhd.extend_from_slice(&1u32.to_be_bytes());
        tkhd.extend_from_slice(&[0u8; 20]);
        tkhd.extend_from_slice(&0x0100u16.to_be_bytes());
        tkhd.extend_from_slice(&0u16.to_be_bytes());
        for n in [0x00010000u32, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000] {
            tkhd.extend_from_slice(&n.to_be_bytes());
        }
        tkhd.extend_from_slice(&((self.width as u32) << 16).to_be_bytes());
        tkhd.extend_from_slice(&((self.height as u32) << 16).to_be_bytes());
        let mut out = full(b"tkhd", 0, 0x7, &tkhd);
        out.extend_from_slice(&bx(b"mdia", &self.mdia()));
        out
    }

    fn mdia(&self) -> Vec<u8> {
        let mut mdhd = Vec::new();
        mdhd.extend_from_slice(&[0u8; 8]);
        mdhd.extend_from_slice(&self.timescale.to_be_bytes());
        mdhd.extend_from_slice(&0u32.to_be_bytes());
        mdhd.extend_from_slice(&0x55c4u16.to_be_bytes());
        mdhd.extend_from_slice(&0u16.to_be_bytes());
        let mut inner = full(b"mdhd", 0, 0, &mdhd);
        let mut hdlr = vec![0u8; 4];
        hdlr.extend_from_slice(b"vide");
        hdlr.extend_from_slice(&[0u8; 12]);
        hdlr.extend_from_slice(b"eiviz\0");
        inner.extend_from_slice(&full(b"hdlr", 0, 0, &hdlr));
        inner.extend_from_slice(&bx(b"minf", &self.minf()));
        inner
    }

    fn minf(&self) -> Vec<u8> {
        let mut inner = full(b"vmhd", 0, 1, &[0u8; 8]);
        let url = full(b"url ", 0, 1, &[]);
        let mut dref = 1u32.to_be_bytes().to_vec();
        dref.extend_from_slice(&url);
        inner.extend_from_slice(&bx(b"dinf", &full(b"dref", 0, 0, &dref)));
        inner.extend_from_slice(&bx(b"stbl", &self.stbl()));
        inner
    }

    fn stbl(&self) -> Vec<u8> {
        let mut avc1 = vec![0u8; 6];
        avc1.extend_from_slice(&1u16.to_be_bytes());
        avc1.extend_from_slice(&[0u8; 16]);
        avc1.extend_from_slice(&self.width.to_be_bytes());
        avc1.extend_from_slice(&self.height.to_be_bytes());
        avc1.extend_from_slice(&0x00480000u32.to_be_bytes());
        avc1.extend_from_slice(&0x00480000u32.to_be_bytes());
        avc1.extend_from_slice(&0u32.to_be_bytes());
        avc1.extend_from_slice(&1u16.to_be_bytes());
        avc1.extend_from_slice(&[0u8; 32]);
        avc1.push(0);
        avc1.extend_from_slice(&0x0018u16.to_be_bytes());
        avc1.extend_from_slice(&0xffffu16.to_be_bytes());
        avc1.extend_from_slice(&bx(b"avcC", &self.avcc));
        let mut stsd = 1u32.to_be_bytes().to_vec();
        stsd.extend_from_slice(&bx(b"avc1", &avc1));
        let mut inner = full(b"stsd", 0, 0, &stsd);
        inner.extend_from_slice(&full(b"stts", 0, 0, &0u32.to_be_bytes()));
        inner.extend_from_slice(&full(b"stsc", 0, 0, &0u32.to_be_bytes()));
        let mut stsz = 0u32.to_be_bytes().to_vec();
        stsz.extend_from_slice(&0u32.to_be_bytes());
        inner.extend_from_slice(&full(b"stsz", 0, 0, &stsz));
        inner.extend_from_slice(&full(b"stco", 0, 0, &0u32.to_be_bytes()));
        inner
    }

    fn moof(&self, duration: u32, size: u32, flags: u32) -> Vec<u8> {
        let mfhd = full(b"mfhd", 0, 0, &self.seq.to_be_bytes());
        let tfhd = full(b"tfhd", 0, 0x020000, &1u32.to_be_bytes());
        let tfdt = full(b"tfdt", 1, 0, &0u64.to_be_bytes());
        let mut trun = 1u32.to_be_bytes().to_vec();
        trun.extend_from_slice(&0u32.to_be_bytes());
        trun.extend_from_slice(&duration.to_be_bytes());
        trun.extend_from_slice(&size.to_be_bytes());
        trun.extend_from_slice(&flags.to_be_bytes());
        let mut traf_inner = tfhd;
        traf_inner.extend_from_slice(&tfdt);
        traf_inner.extend_from_slice(&full(b"trun", 1, 0x000701, &trun));
        let mut inner = mfhd;
        inner.extend_from_slice(&bx(b"traf", &traf_inner));
        bx(b"moof", &inner)
    }
}

fn avcc_from_sps_pps(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut v = vec![1, 66, 0, 31, 0xff, 0xe1];
    v.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    v.extend_from_slice(sps);
    v.push(1);
    v.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    v.extend_from_slice(pps);
    v
}

fn bx(ty: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = ((8 + payload.len()) as u32).to_be_bytes().to_vec();
    v.extend_from_slice(ty);
    v.extend_from_slice(payload);
    v
}

fn full(ty: &[u8; 4], ver: u8, flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = vec![
        ver,
        ((flags >> 16) & 0xff) as u8,
        ((flags >> 8) & 0xff) as u8,
        (flags & 0xff) as u8,
    ];
    body.extend_from_slice(payload);
    bx(ty, &body)
}
