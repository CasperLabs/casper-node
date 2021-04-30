use crate::bytesrepr::{FromBytes, ToBytes};

/// The quality of having a tag
pub trait Tagged {
    /// Concrete type of the tag itself
    type Tag: ToBytes + FromBytes;

    /// Returns the tag of a given object
    fn tag(&self) -> Self::Tag;
}
