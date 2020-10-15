pub use scones_macros::*;
use std::marker::PhantomData;

/// Indicates that a particular required value has been provided in a builder.
pub struct Present;
/// Indicates that a particular required value has not been provided yet in a builder.
pub struct Missing;
#[doc(hidden)]
/// Used to implement builders.
pub struct BuilderFieldContainer<FieldType, IsPresent> {
    data: Option<FieldType>,
    marker_: PhantomData<IsPresent>,
}

impl<FieldType, IsPresent> BuilderFieldContainer<FieldType, IsPresent> {
    pub fn set(self, value: FieldType) -> BuilderFieldContainer<FieldType, Present> {
        BuilderFieldContainer {
            data: Some(value),
            marker_: PhantomData,
        }
    }
}

impl<FieldType> BuilderFieldContainer<FieldType, Missing> {
    pub fn missing() -> Self {
        Self {
            data: None,
            marker_: PhantomData,
        }
    }
}

impl<FieldType> BuilderFieldContainer<FieldType, Present> {
    pub fn present(value: FieldType) -> Self {
        Self {
            data: Some(value),
            marker_: PhantomData,
        }
    }

    pub fn into_value(self) -> FieldType {
        // The only way for IsPresent to be Present is if the user called set() in the past.
        self.data.unwrap()
    }
}
