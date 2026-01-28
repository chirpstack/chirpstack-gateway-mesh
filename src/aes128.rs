use std::fmt;
use std::str::FromStr;

use aes::{
    Aes128, Block,
    cipher::{BlockEncrypt, KeyInit},
};
use anyhow::{Error, Result};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, Visitor},
};

#[derive(Copy, Clone, PartialEq, Eq, Default)]
pub struct Aes128Key([u8; 16]);

impl Aes128Key {
    pub fn null() -> Self {
        Aes128Key([0; 16])
    }

    pub fn from_slice(b: &[u8]) -> Result<Self, Error> {
        if b.len() != 16 {
            return Err(anyhow!("16 bytes are expected"));
        }

        let mut bb: [u8; 16] = [0; 16];
        bb.copy_from_slice(b);

        Ok(Aes128Key(bb))
    }

    pub fn from_bytes(b: [u8; 16]) -> Self {
        Aes128Key(b)
    }

    pub fn to_bytes(&self) -> [u8; 16] {
        self.0
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }
}

impl fmt::Display for Aes128Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

impl fmt::Debug for Aes128Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

impl FromStr for Aes128Key {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut bytes: [u8; 16] = [0; 16];
        hex::decode_to_slice(s, &mut bytes)?;
        Ok(Aes128Key(bytes))
    }
}

impl Serialize for Aes128Key {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Aes128Key {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(Aes128KeyVisitor)
    }
}

struct Aes128KeyVisitor;

impl Visitor<'_> for Aes128KeyVisitor {
    type Value = Aes128Key;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("A hex encoded AES key of 128 bit is expected")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Aes128Key::from_str(value).map_err(|e| E::custom(format!("{}", e)))
    }
}

pub fn get_signing_key(key: Aes128Key) -> Aes128Key {
    let b: [u8; 16] = [0; 16];
    get_key(key, b)
}

pub fn get_encryption_key(key: Aes128Key) -> Aes128Key {
    let mut b: [u8; 16] = [0; 16];
    b[0] = 0x01;
    get_key(key, b)
}

fn get_key(key: Aes128Key, b: [u8; 16]) -> Aes128Key {
    let key_bytes = key.to_bytes();
    let cipher = Aes128::new_from_slice(&key_bytes).expect("Invalid key length");

    let mut block = Block::from(b);
    cipher.encrypt_block(&mut block);

    Aes128Key(block.into())
}
