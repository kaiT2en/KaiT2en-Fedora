// SPDX-License-Identifier: GPL-3.0-or-later
//! UKEY2 handshake for both roles and the D2D secure channel on top of it.
//!
//! Key derivation and message layout follow NearDrop PROTOCOL.md and are
//! adapted from rquickshare `core_lib/src/hdl/inbound.rs` and
//! `core_lib/src/utils.rs` (commit 378d8ae, GPL-3.0), see
//! THIRD-PARTY-NOTICES.md.

use aes::Aes256;
use anyhow::{Context, Result, anyhow, bail};
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use p256::elliptic_curve::rand_core::OsRng;
use p256::elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint};
use p256::{EncodedPoint, PublicKey, SecretKey};
use prost::Message;
use rand::RngCore;
use sha2::{Digest, Sha256, Sha512};

use crate::proto::location::nearby::connections::OfflineFrame;
use crate::proto::securegcm::{
    DeviceToDeviceMessage, GcmMetadata, Type as GcmType, Ukey2ClientFinished, Ukey2ClientInit,
    Ukey2HandshakeCipher, Ukey2Message, Ukey2ServerInit, ukey2_client_init, ukey2_message,
};
use crate::proto::securemessage::{
    EcP256PublicKey, EncScheme, GenericPublicKey, Header, HeaderAndBody, PublicKeyType,
    SecureMessage, SigScheme,
};

type HmacSha256 = Hmac<Sha256>;

const NEXT_PROTOCOL: &str = "AES_256_CBC-HMAC_SHA256";
/// SHA256("D2D")
const D2D_SALT: [u8; 32] = hex32("82aa55a0d397f88346ca1cee8d3909b95f13fa7deb1d4ab38376b8256da85510");
/// SHA256("SecureMessage")
const SECUREMESSAGE_SALT: [u8; 32] =
    hex32("bf9d2a53c63616d75db0a7165b91c1ef73e537f2427405fa23610a4be657642e");

/// Server side of the UKEY2 handshake after ClientInit has been answered.
pub struct ServerHandshake {
    secret: SecretKey,
    client_init: Vec<u8>,
    server_init: Vec<u8>,
    commitment: Vec<u8>,
}

impl ServerHandshake {
    /// Checks the raw ClientInit message and returns the handshake state
    /// together with the raw ServerInit message to send.
    pub fn accept(client_init_raw: &[u8]) -> Result<(Self, Vec<u8>)> {
        let msg = Ukey2Message::decode(client_init_raw).context("UKEY2 message")?;
        if msg.message_type() != ukey2_message::Type::ClientInit {
            bail!("UKEY2: expected ClientInit, got {:?}", msg.message_type());
        }
        let init = Ukey2ClientInit::decode(msg.message_data()).context("UKEY2 ClientInit")?;
        if init.version() != 1 {
            bail!("UKEY2: ClientInit version {}", init.version());
        }
        if init.random().len() != 32 {
            bail!("UKEY2: ClientInit random has {} bytes", init.random().len());
        }
        if init.next_protocol() != NEXT_PROTOCOL {
            bail!("UKEY2: next protocol {}", init.next_protocol());
        }
        let commitment = init
            .cipher_commitments
            .iter()
            .find(|c| c.handshake_cipher() == Ukey2HandshakeCipher::P256Sha512)
            .map(|c| c.commitment().to_vec())
            .ok_or_else(|| anyhow!("UKEY2: no P256_SHA512 commitment"))?;

        let (secret, public_key) = new_key()?;
        let server_init = Ukey2ServerInit {
            version: Some(1),
            random: Some(random32()),
            handshake_cipher: Some(Ukey2HandshakeCipher::P256Sha512.into()),
            public_key: Some(public_key.encode_to_vec()),
        };
        let server_init = Ukey2Message {
            message_type: Some(ukey2_message::Type::ServerInit.into()),
            message_data: Some(server_init.encode_to_vec()),
        }
        .encode_to_vec();

        let state = Self {
            secret,
            client_init: client_init_raw.to_vec(),
            server_init: server_init.clone(),
            commitment,
        };
        Ok((state, server_init))
    }

    /// Checks the raw ClientFinished message against the commitment and
    /// derives the channel keys. Returns the channel and the 4 digit PIN
    /// both sides show.
    pub fn finish(self, client_finish_raw: &[u8]) -> Result<(SecureChannel, String)> {
        if Sha512::digest(client_finish_raw).as_slice() != self.commitment.as_slice() {
            bail!("UKEY2: ClientFinished does not match the commitment");
        }
        let msg = Ukey2Message::decode(client_finish_raw).context("UKEY2 message")?;
        if msg.message_type() != ukey2_message::Type::ClientFinish {
            bail!("UKEY2: expected ClientFinish, got {:?}", msg.message_type());
        }
        let finish =
            Ukey2ClientFinished::decode(msg.message_data()).context("UKEY2 ClientFinished")?;
        let peer = peer_key(finish.public_key())?;
        derive(Role::Server, &self.secret, &peer, &self.client_init, &self.server_init)
    }
}

/// Client side of the UKEY2 handshake after ClientInit has been sent.
pub struct ClientHandshake {
    secret: SecretKey,
    client_init: Vec<u8>,
    client_finish: Vec<u8>,
}

impl ClientHandshake {
    /// Returns the handshake state and the raw ClientInit message to send.
    /// ClientFinished is built up front because ClientInit commits to its
    /// hash.
    pub fn start() -> Result<(Self, Vec<u8>)> {
        let (secret, public_key) = new_key()?;
        let client_finish = Ukey2Message {
            message_type: Some(ukey2_message::Type::ClientFinish.into()),
            message_data: Some(
                Ukey2ClientFinished { public_key: Some(public_key.encode_to_vec()) }
                    .encode_to_vec(),
            ),
        }
        .encode_to_vec();
        let client_init = Ukey2Message {
            message_type: Some(ukey2_message::Type::ClientInit.into()),
            message_data: Some(
                Ukey2ClientInit {
                    version: Some(1),
                    random: Some(random32()),
                    cipher_commitments: vec![ukey2_client_init::CipherCommitment {
                        handshake_cipher: Some(Ukey2HandshakeCipher::P256Sha512.into()),
                        commitment: Some(Sha512::digest(&client_finish).to_vec()),
                    }],
                    next_protocol: Some(NEXT_PROTOCOL.to_string()),
                }
                .encode_to_vec(),
            ),
        }
        .encode_to_vec();
        let state = Self { secret, client_init: client_init.clone(), client_finish };
        Ok((state, client_init))
    }

    /// Checks the raw ServerInit message and derives the channel keys.
    /// Returns the channel, the PIN and the raw ClientFinished to send.
    pub fn finish(self, server_init_raw: &[u8]) -> Result<(SecureChannel, String, Vec<u8>)> {
        let msg = Ukey2Message::decode(server_init_raw).context("UKEY2 message")?;
        if msg.message_type() != ukey2_message::Type::ServerInit {
            bail!("UKEY2: expected ServerInit, got {:?}", msg.message_type());
        }
        let init = Ukey2ServerInit::decode(msg.message_data()).context("UKEY2 ServerInit")?;
        if init.version() != 1 {
            bail!("UKEY2: ServerInit version {}", init.version());
        }
        if init.random().len() != 32 {
            bail!("UKEY2: ServerInit random has {} bytes", init.random().len());
        }
        if init.handshake_cipher() != Ukey2HandshakeCipher::P256Sha512 {
            bail!("UKEY2: handshake cipher {:?}", init.handshake_cipher());
        }
        let peer = peer_key(init.public_key())?;
        let (channel, pin) =
            derive(Role::Client, &self.secret, &peer, &self.client_init, server_init_raw)?;
        Ok((channel, pin, self.client_finish))
    }
}

#[derive(Clone, Copy)]
enum Role {
    Client,
    Server,
}

fn new_key() -> Result<(SecretKey, GenericPublicKey)> {
    let secret = SecretKey::random(&mut OsRng);
    let point = secret.public_key().to_encoded_point(false);
    let public_key = GenericPublicKey {
        r#type: PublicKeyType::EcP256.into(),
        ec_p256_public_key: Some(EcP256PublicKey {
            x: signed_be(point.x().ok_or_else(|| anyhow!("no x"))?),
            y: signed_be(point.y().ok_or_else(|| anyhow!("no y"))?),
        }),
        ..Default::default()
    };
    Ok((secret, public_key))
}

fn random32() -> Vec<u8> {
    let mut random = [0u8; 32];
    OsRng.fill_bytes(&mut random);
    random.to_vec()
}

/// The peer's P-256 key from a serialized GenericPublicKey.
fn peer_key(raw: &[u8]) -> Result<PublicKey> {
    let key = GenericPublicKey::decode(raw).context("peer public key")?;
    let key = key.ec_p256_public_key.ok_or_else(|| anyhow!("peer key is not P-256"))?;
    let mut sec1 = vec![0x04];
    sec1.extend_from_slice(&unsigned_32(&key.x)?);
    sec1.extend_from_slice(&unsigned_32(&key.y)?);
    let point = EncodedPoint::from_bytes(&sec1).context("peer key point")?;
    Option::<PublicKey>::from(PublicKey::from_encoded_point(&point))
        .ok_or_else(|| anyhow!("peer key is not on the curve"))
}

/// Channel keys and PIN from the ECDH secret and both raw init messages.
/// The client uses the server's keys for receiving and its own for
/// sending, the server the other way round.
fn derive(
    role: Role,
    secret: &SecretKey,
    peer: &PublicKey,
    client_init: &[u8],
    server_init: &[u8],
) -> Result<(SecureChannel, String)> {
    let shared = p256::ecdh::diffie_hellman(secret.to_nonzero_scalar(), peer.as_affine());
    let derived = Sha256::digest(shared.raw_secret_bytes());
    let mut info = client_init.to_vec();
    info.extend_from_slice(server_init);

    let auth = hkdf(b"UKEY2 v1 auth", &derived, &info);
    let next = hkdf(b"UKEY2 v1 next", &derived, &info);
    let d2d_client = hkdf(&D2D_SALT, &next, b"client");
    let d2d_server = hkdf(&D2D_SALT, &next, b"server");
    let (receive_from, send_with) = match role {
        Role::Server => (d2d_client, d2d_server),
        Role::Client => (d2d_server, d2d_client),
    };
    let channel = SecureChannel {
        decrypt_key: hkdf(&SECUREMESSAGE_SALT, &receive_from, b"ENC:2"),
        recv_hmac_key: hkdf(&SECUREMESSAGE_SALT, &receive_from, b"SIG:1"),
        encrypt_key: hkdf(&SECUREMESSAGE_SALT, &send_with, b"ENC:2"),
        send_hmac_key: hkdf(&SECUREMESSAGE_SALT, &send_with, b"SIG:1"),
        send_seq: 0,
        recv_seq: 0,
    };
    Ok((channel, pin_code(&auth)))
}

/// Encrypted and signed transport for offline frames.
pub struct SecureChannel {
    decrypt_key: [u8; 32],
    recv_hmac_key: [u8; 32],
    encrypt_key: [u8; 32],
    send_hmac_key: [u8; 32],
    send_seq: i32,
    recv_seq: i32,
}

impl SecureChannel {
    /// Wraps a frame into a signed SecureMessage, returns its wire bytes.
    pub fn seal(&mut self, frame: &OfflineFrame) -> Vec<u8> {
        self.send_seq += 1;
        let d2d = DeviceToDeviceMessage {
            sequence_number: Some(self.send_seq),
            message: Some(frame.encode_to_vec()),
        };
        let mut iv = [0u8; 16];
        OsRng.fill_bytes(&mut iv);
        let body = cbc::Encryptor::<Aes256>::new(&self.encrypt_key.into(), &iv.into())
            .encrypt_padded_vec_mut::<Pkcs7>(&d2d.encode_to_vec());
        let header_and_body = HeaderAndBody {
            header: Header {
                signature_scheme: SigScheme::HmacSha256.into(),
                encryption_scheme: EncScheme::Aes256Cbc.into(),
                iv: Some(iv.to_vec()),
                public_metadata: Some(
                    GcmMetadata {
                        r#type: GcmType::DeviceToDeviceMessage.into(),
                        version: Some(1),
                    }
                    .encode_to_vec(),
                ),
                ..Default::default()
            },
            body,
        }
        .encode_to_vec();
        let mut mac = HmacSha256::new_from_slice(&self.send_hmac_key).expect("HMAC key length");
        mac.update(&header_and_body);
        SecureMessage {
            header_and_body,
            signature: mac.finalize().into_bytes().to_vec(),
        }
        .encode_to_vec()
    }

    /// Verifies and decrypts a SecureMessage, checks the sequence number and
    /// returns the offline frame inside.
    pub fn open(&mut self, raw: &[u8]) -> Result<OfflineFrame> {
        let msg = SecureMessage::decode(raw).context("SecureMessage")?;
        let mut mac = HmacSha256::new_from_slice(&self.recv_hmac_key).expect("HMAC key length");
        mac.update(&msg.header_and_body);
        mac.verify_slice(&msg.signature)
            .map_err(|_| anyhow!("SecureMessage signature mismatch"))?;
        let header_and_body = HeaderAndBody::decode(msg.header_and_body.as_slice())?;
        let iv: [u8; 16] = header_and_body
            .header
            .iv()
            .try_into()
            .map_err(|_| anyhow!("SecureMessage IV is not 16 bytes"))?;
        let plain = cbc::Decryptor::<Aes256>::new(&self.decrypt_key.into(), &iv.into())
            .decrypt_padded_vec_mut::<Pkcs7>(&header_and_body.body)
            .map_err(|_| anyhow!("SecureMessage padding"))?;
        let d2d = DeviceToDeviceMessage::decode(plain.as_slice())?;
        self.recv_seq += 1;
        if d2d.sequence_number() != self.recv_seq {
            bail!(
                "sequence number {} where {} was expected",
                d2d.sequence_number(),
                self.recv_seq
            );
        }
        Ok(OfflineFrame::decode(d2d.message())?)
    }
}

fn hkdf(salt: &[u8], ikm: &[u8], info: &[u8]) -> [u8; 32] {
    let mut okm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(salt), ikm)
        .expand(info, &mut okm)
        .expect("32 bytes is a valid HKDF-SHA256 length");
    okm
}

/// Big-endian two's complement as Java's BigInteger writes it: a leading
/// zero byte when the top bit is set.
fn signed_be(unsigned: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(unsigned.len() + 1);
    if unsigned.first().is_some_and(|b| b & 0x80 != 0) {
        out.push(0);
    }
    out.extend_from_slice(unsigned);
    out
}

/// The 32 byte coordinate from a signed big-endian integer, which may carry
/// a leading zero byte or be shorter than 32 bytes.
fn unsigned_32(signed: &[u8]) -> Result<[u8; 32]> {
    let trimmed: &[u8] = match signed.iter().position(|&b| b != 0) {
        Some(start) => &signed[start..],
        None => &[],
    };
    if trimmed.len() > 32 {
        bail!("coordinate has {} bytes", trimmed.len());
    }
    let mut out = [0u8; 32];
    out[32 - trimmed.len()..].copy_from_slice(trimmed);
    Ok(out)
}

/// Nearby Share's 4 digit PIN from the UKEY2 auth string.
fn pin_code(auth: &[u8]) -> String {
    const MODULO: i32 = 9973;
    const MULTIPLIER: i32 = 31;
    let mut hash = 0i32;
    let mut factor = 1i32;
    for &byte in auth {
        hash = (hash + byte as i8 as i32 * factor) % MODULO;
        factor = (factor * MULTIPLIER) % MODULO;
    }
    format!("{:04}", hash.abs())
}

const fn hex32(s: &str) -> [u8; 32] {
    const fn nibble(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("not a lower case hex digit"),
        }
    }
    let bytes = s.as_bytes();
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = nibble(bytes[2 * i]) << 4 | nibble(bytes[2 * i + 1]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salts_are_the_documented_hashes() {
        assert_eq!(D2D_SALT.as_slice(), Sha256::digest(b"D2D").as_slice());
        assert_eq!(SECUREMESSAGE_SALT.as_slice(), Sha256::digest(b"SecureMessage").as_slice());
    }

    #[test]
    fn client_and_server_agree() {
        let (client, client_init) = ClientHandshake::start().unwrap();
        let (server, server_init) = ServerHandshake::accept(&client_init).unwrap();
        let (mut client_channel, client_pin, client_finish) = client.finish(&server_init).unwrap();
        let (mut server_channel, server_pin) = server.finish(&client_finish).unwrap();
        assert_eq!(client_pin, server_pin);

        let frame = OfflineFrame { version: Some(1), ..Default::default() };
        let sealed = client_channel.seal(&frame);
        assert_eq!(server_channel.open(&sealed).unwrap().version, Some(1));
        let sealed = server_channel.seal(&frame);
        assert_eq!(client_channel.open(&sealed).unwrap().version, Some(1));
    }

    #[test]
    fn coordinates_round_trip() {
        let mut x = [0u8; 32];
        x[0] = 0x80;
        x[31] = 1;
        assert_eq!(signed_be(&x).len(), 33);
        assert_eq!(unsigned_32(&signed_be(&x)).unwrap(), x);
        assert_eq!(unsigned_32(&[1, 2]).unwrap()[30..], [1, 2]);
    }
}
