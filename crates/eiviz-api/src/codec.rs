use prost::Message;

use crate::proto::Envelope;

pub const WS_SUBPROTOCOL: &str = "eiviz.protobuf.v1";
pub const TCP_MAGIC: &[u8; 4] = b"EIVZ";
pub const TCP_VERSION: u8 = 1;
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

pub fn encode_envelope(envelope: &Envelope) -> Vec<u8> {
    let mut buf = Vec::new();
    envelope.encode(&mut buf).expect("encode envelope");
    buf
}

pub fn decode_envelope(bytes: &[u8]) -> Result<Envelope, String> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err("message too large".into());
    }
    Envelope::decode(bytes).map_err(|error| error.to_string())
}

pub fn encode_tcp_frame(envelope: &Envelope) -> Vec<u8> {
    let payload = encode_envelope(envelope);
    let mut out = Vec::with_capacity(9 + payload.len());
    out.extend_from_slice(TCP_MAGIC);
    out.push(TCP_VERSION);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(&payload);
    out
}

pub fn decode_tcp_header(header: &[u8; 9]) -> Result<usize, String> {
    if &header[0..4] != TCP_MAGIC {
        return Err("bad tcp magic".into());
    }
    if header[4] != TCP_VERSION {
        return Err("unsupported tcp protocol version".into());
    }
    let len = u32::from_be_bytes(header[5..9].try_into().unwrap()) as usize;
    if len == 0 || len > MAX_MESSAGE_BYTES {
        return Err("invalid tcp frame length".into());
    }
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{ClientHello, Envelope, envelope};

    #[test]
    fn tcp_roundtrip_and_reject_bad_magic() {
        let env = Envelope {
            kind: Some(envelope::Kind::Hello(ClientHello {
                protocol: WS_SUBPROTOCOL.into(),
                ..Default::default()
            })),
        };
        let frame = encode_tcp_frame(&env);
        let mut header = [0u8; 9];
        header.copy_from_slice(&frame[..9]);
        let len = decode_tcp_header(&header).unwrap();
        let decoded = decode_envelope(&frame[9..9 + len]).unwrap();
        assert_eq!(decoded.kind.is_some(), true);
        let mut bad = header;
        bad[0] = b'X';
        assert!(decode_tcp_header(&bad).is_err());
    }
}
