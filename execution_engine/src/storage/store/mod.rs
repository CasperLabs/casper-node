mod store_ext;
#[cfg(test)]
pub(crate) mod tests;

use casper_types::bytesrepr::{self, FromBytes, ToBytes};

pub use self::store_ext::StoreExt;
use crate::storage::transaction_source::{Readable, Writable};

pub trait Store<K, V> {
    type Error: From<bytesrepr::Error>;

    type Handle;

    fn handle(&self) -> Self::Handle;

    fn deserialize_value(&self, _key: &K, bytes: Vec<u8>) -> Result<V, bytesrepr::Error>
    where
        V: FromBytes,
    {
        bytesrepr::deserialize(bytes)
    }

    fn serialize_value(&self, _key: &K, value: &V) -> Result<Vec<u8>, bytesrepr::Error>
    where
        V: ToBytes,
    {
        value.to_bytes()
    }

    fn get<T>(&self, txn: &T, key: &K) -> Result<Option<V>, Self::Error>
    where
        T: Readable<Handle = Self::Handle>,
        K: ToBytes,
        V: FromBytes,
        Self::Error: From<T::Error>,
    {
        let handle = self.handle();
        match txn.read(handle, &key.to_bytes()?)? {
            None => Ok(None),
            Some(value_bytes) => {
                let value = self.deserialize_value(key, value_bytes.into())?;
                Ok(Some(value))
            }
        }
    }

    fn put<T>(&self, txn: &mut T, key: &K, value: &V) -> Result<(), Self::Error>
    where
        T: Writable<Handle = Self::Handle>,
        K: ToBytes,
        V: ToBytes,
        Self::Error: From<T::Error>,
    {
        let handle = self.handle();
        let key_bytes = key.to_bytes()?;
        let value_bytes = self.serialize_value(key, value)?;
        txn.write(handle, &key_bytes, &value_bytes)
            .map_err(Into::into)
    }
}
