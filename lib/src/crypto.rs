use ecdsa::{
    signature::{Signer, Verifier},
    Signature as ECDSASignature, SigningKey, VerifyingKey,
};

use k256::Secp256k1;
use serde::{Deserialize, Serialize};

use crate::sha256::Hash;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Signature(ECDSASignature<Secp256k1>);

impl Signature {
    /// Sign a TxOutput from its SHA256 hash
    pub fn sign_output(output_hash: &Hash, privkey: &PrivateKey) -> Self {
        let signing_key = &privkey.0;
        let signature = signing_key.sign(&output_hash.as_bytes());
        Self(signature)
    }

    /// Verify a signature of an output
    pub fn verify(&self, output_hash: &Hash, pubkey: &PublicKey) -> bool {
        pubkey.0.verify(&output_hash.as_bytes(), &self.0).is_ok()
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct PublicKey(VerifyingKey<Secp256k1>);

impl std::fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("PublicKey")
            .field(&self.0.to_encoded_point(true).to_string())
            .finish()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PrivateKey(#[serde(with = "signkey_serde")] SigningKey<Secp256k1>);

impl PrivateKey {
    /// Generates new private key
    pub fn new_key() -> Self {
        Self(SigningKey::random(&mut rand::thread_rng()))
    }

    /// Corresponding public key
    pub fn public_key(&self) -> PublicKey {
        PublicKey(*self.0.verifying_key())
    }
}

mod signkey_serde {
    use super::{Secp256k1, SigningKey};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(key: &SigningKey<Secp256k1>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(&key.to_bytes())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SigningKey<Secp256k1>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = Vec::<u8>::deserialize(deserializer)?;
        Ok(SigningKey::from_slice(&bytes).expect("failed to deserialize signing key"))
    }
}
