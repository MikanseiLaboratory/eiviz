/// Big-endian bit writer for H.264 RBSP.
#[derive(Default)]
pub struct BitWriter {
    pub bytes: Vec<u8>,
    acc: u8,
    nbits: u8,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn write_bit(&mut self, bit: u8) {
        self.acc = (self.acc << 1) | (bit & 1);
        self.nbits += 1;
        if self.nbits == 8 {
            self.bytes.push(self.acc);
            self.acc = 0;
            self.nbits = 0;
        }
    }

    pub fn write_bits(&mut self, value: u32, n: u32) {
        for i in (0..n).rev() {
            self.write_bit(((value >> i) & 1) as u8);
        }
    }

    pub fn write_ue(&mut self, v: u32) {
        let x = v + 1;
        let lz = 31 - x.leading_zeros();
        for _ in 0..lz {
            self.write_bit(0);
        }
        self.write_bits(x, lz + 1);
    }

    pub fn write_se(&mut self, v: i32) {
        let ue = if v <= 0 {
            ((-v) as u32) << 1
        } else {
            ((v as u32) << 1) - 1
        };
        self.write_ue(ue);
    }

    pub fn align_byte(&mut self) {
        while self.nbits != 0 {
            self.write_bit(0);
        }
    }

    pub fn rbsp_trailing(&mut self) {
        self.write_bit(1);
        self.align_byte();
    }

    pub fn into_rbsp(mut self) -> Vec<u8> {
        self.rbsp_trailing();
        self.bytes
    }
}

pub fn annexb(nal_type: u8, nal_ref_idc: u8, rbsp: &[u8]) -> Vec<u8> {
    let mut body = vec![(nal_ref_idc << 5) | (nal_type & 0x1f)];
    let mut zeros = 0u8;
    for &b in rbsp {
        if zeros >= 2 && b <= 3 {
            body.push(0x03);
            zeros = 0;
        }
        body.push(b);
        zeros = if b == 0 { zeros + 1 } else { 0 };
    }
    let mut out = vec![0, 0, 0, 1];
    out.extend_from_slice(&body);
    out
}
