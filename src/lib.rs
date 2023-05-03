use serde::{Deserialize, Serialize};
use thiserror::Error;
use ed25519_dalek::{
    Signer,
    SigningKey,
};

pub type Label = u16;
#[derive(Error, Debug)]
pub enum LabelError{
    #[error("serialization error")]
    Serialization(#[from] ciborium::ser::Error<std::io::Error>),
    #[error("deserialization error")]
    Deserialization(#[from] ciborium::de::Error<std::io::Error>),
}

pub struct ErrorCounter{
    count: usize,
}

impl ErrorCounter {
    pub fn new() -> Self{
        ErrorCounter{
            count:0
        }
    }
    pub fn inc(&mut self) {
        self.count += 1;
    }
    pub fn reset(&mut self) {
        self.count = 0;
    }
    pub fn is_too_mutch(&self) -> bool{
        self.count > 40
    }
}

pub struct Key {
    secret: ed25519_dalek::SigningKey,
    id: String,
    id_len: usize ,
}

impl Key {
    pub fn new(secret: SigningKey, id: String) -> Self{
        let id_len = id.as_bytes().len();
        Key{
            secret,
            id,
            id_len,
        }
    }
    pub fn sign_with_ad(&self, payload: Vec<u8>, ad: Vec<u8>) -> SignedMsg {
        let mut buffer:Vec<u8> = Vec::with_capacity(payload.len() + ad.len()+ std::mem::size_of::<i64>() + self.id_len);
        let datetime = chrono::Utc::now().timestamp();
        buffer.extend_from_slice(&payload);
        buffer.extend_from_slice(&ad);
        buffer.extend_from_slice(&datetime.to_be_bytes());
        buffer.extend_from_slice(self.id.as_bytes());
        let signature = self.secret.sign(&buffer).to_vec();
        SignedMsg {
            payload,
            ad,
            key_id: self.id.clone(),
            datetime,
            signature,
        }
    }

    pub fn sign(&self, data: Vec<u8>) -> SignedMsg{
        let ad = Vec::new();
        self.sign_with_ad(data, ad)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SignedMsg {
    payload: Vec<u8>,
    ad: Vec<u8>,
    key_id: String,
    datetime: i64,
    signature: Vec<u8>,
}

impl SignedMsg{
    pub fn verify(&self, key: &Key)  {

    }
    pub fn get_key_id(&self) -> &str {
        &self.key_id
    }
}


#[derive(Debug, Serialize, Deserialize)]
pub struct AD {
    pub key_id: String,
    pub datetime: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LabeledInfo {
    pub topic: String,
    pub label: Label,
}
impl LabeledInfo {
    pub fn new(topic: &str, label: Label) -> Self {
        LabeledInfo {
            topic: topic.into(),
            label,
        }
    }
    
    pub fn serialize(&self) -> Result<Vec<u8>, LabelError> {
        let mut label_info_bytes = Vec::new();
        ciborium::ser::into_writer(self, &mut label_info_bytes)?;
        Ok(label_info_bytes)
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, LabelError> {
        Ok(ciborium::de::from_reader(bytes)?)
    }
}

