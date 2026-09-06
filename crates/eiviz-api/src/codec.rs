use prost::Message;

use crate::proto::Envelope;

pub const WS_SUBPROTOCOL: &str = "eiviz.protobuf.v1";
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{ClientHello, Envelope, envelope};

    #[test]
    fn envelope_roundtrip() {
        let env = Envelope {
            kind: Some(envelope::Kind::Hello(ClientHello {
                protocol: WS_SUBPROTOCOL.into(),
                ..Default::default()
            })),
        };
        let bytes = encode_envelope(&env);
        let decoded = decode_envelope(&bytes).unwrap();
        assert!(decoded.kind.is_some());
        assert!(decode_envelope(&vec![0; MAX_MESSAGE_BYTES + 1]).is_err());
    }
}
