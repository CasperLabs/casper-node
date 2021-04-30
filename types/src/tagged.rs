/// The quality of having a tag
pub trait Tagged {
    /// Concrete type of the tag itself
    type Tag;

    /// Returns the tag of a given object
    fn tag(&self) -> Self::Tag;
}
