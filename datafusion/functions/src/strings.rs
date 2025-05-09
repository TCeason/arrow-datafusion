// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

use std::mem::size_of;

use arrow::array::{
    make_view, Array, ArrayAccessor, ArrayDataBuilder, ArrayIter, ByteView,
    GenericStringArray, LargeStringArray, NullBufferBuilder, OffsetSizeTrait,
    StringArray, StringViewArray, StringViewBuilder,
};
use arrow::buffer::{MutableBuffer, NullBuffer};
use arrow::datatypes::DataType;

/// Abstracts iteration over different types of string arrays.
#[deprecated(since = "45.0.0", note = "Use arrow::array::StringArrayType instead")]
pub trait StringArrayType<'a>: ArrayAccessor<Item = &'a str> + Sized {
    /// Return an [`ArrayIter`]  over the values of the array.
    ///
    /// This iterator iterates returns `Option<&str>` for each item in the array.
    fn iter(&self) -> ArrayIter<Self>;

    /// Check if the array is ASCII only.
    fn is_ascii(&self) -> bool;
}

#[allow(deprecated)]
impl<'a, T: OffsetSizeTrait> StringArrayType<'a> for &'a GenericStringArray<T> {
    fn iter(&self) -> ArrayIter<Self> {
        GenericStringArray::<T>::iter(self)
    }

    fn is_ascii(&self) -> bool {
        GenericStringArray::<T>::is_ascii(self)
    }
}

#[allow(deprecated)]
impl<'a> StringArrayType<'a> for &'a StringViewArray {
    fn iter(&self) -> ArrayIter<Self> {
        StringViewArray::iter(self)
    }

    fn is_ascii(&self) -> bool {
        StringViewArray::is_ascii(self)
    }
}

/// Optimized version of the StringBuilder in Arrow that:
/// 1. Precalculating the expected length of the result, avoiding reallocations.
/// 2. Avoids creating / incrementally creating a `NullBufferBuilder`
pub struct StringArrayBuilder {
    offsets_buffer: MutableBuffer,
    value_buffer: MutableBuffer,
}

impl StringArrayBuilder {
    pub fn with_capacity(item_capacity: usize, data_capacity: usize) -> Self {
        let capacity = item_capacity
            .checked_add(1)
            .map(|i| i.saturating_mul(size_of::<i32>()))
            .expect("capacity integer overflow");

        let mut offsets_buffer = MutableBuffer::with_capacity(capacity);
        // SAFETY: the first offset value is definitely not going to exceed the bounds.
        unsafe { offsets_buffer.push_unchecked(0_i32) };
        Self {
            offsets_buffer,
            value_buffer: MutableBuffer::with_capacity(data_capacity),
        }
    }

    pub fn write<const CHECK_VALID: bool>(
        &mut self,
        column: &ColumnarValueRef,
        i: usize,
    ) {
        match column {
            ColumnarValueRef::Scalar(s) => {
                self.value_buffer.extend_from_slice(s);
            }
            ColumnarValueRef::NullableArray(array) => {
                if !CHECK_VALID || array.is_valid(i) {
                    self.value_buffer
                        .extend_from_slice(array.value(i).as_bytes());
                }
            }
            ColumnarValueRef::NullableLargeStringArray(array) => {
                if !CHECK_VALID || array.is_valid(i) {
                    self.value_buffer
                        .extend_from_slice(array.value(i).as_bytes());
                }
            }
            ColumnarValueRef::NullableStringViewArray(array) => {
                if !CHECK_VALID || array.is_valid(i) {
                    self.value_buffer
                        .extend_from_slice(array.value(i).as_bytes());
                }
            }
            ColumnarValueRef::NonNullableArray(array) => {
                self.value_buffer
                    .extend_from_slice(array.value(i).as_bytes());
            }
            ColumnarValueRef::NonNullableLargeStringArray(array) => {
                self.value_buffer
                    .extend_from_slice(array.value(i).as_bytes());
            }
            ColumnarValueRef::NonNullableStringViewArray(array) => {
                self.value_buffer
                    .extend_from_slice(array.value(i).as_bytes());
            }
        }
    }

    pub fn append_offset(&mut self) {
        let next_offset: i32 = self
            .value_buffer
            .len()
            .try_into()
            .expect("byte array offset overflow");
        self.offsets_buffer.push(next_offset);
    }

    /// Finalize the builder into a concrete [`StringArray`].
    ///
    /// # Panics
    ///
    /// This method can panic when:
    ///
    /// - the provided `null_buffer` is not the same length as the `offsets_buffer`.
    pub fn finish(self, null_buffer: Option<NullBuffer>) -> StringArray {
        let row_count = self.offsets_buffer.len() / size_of::<i32>() - 1;
        if let Some(ref null_buffer) = null_buffer {
            assert_eq!(
                null_buffer.len(),
                row_count,
                "Null buffer and offsets buffer must be the same length"
            );
        }
        let array_builder = ArrayDataBuilder::new(DataType::Utf8)
            .len(row_count)
            .add_buffer(self.offsets_buffer.into())
            .add_buffer(self.value_buffer.into())
            .nulls(null_buffer);
        // SAFETY: all data that was appended was valid UTF8 and the values
        // and offsets were created correctly
        let array_data = unsafe { array_builder.build_unchecked() };
        StringArray::from(array_data)
    }
}

pub struct StringViewArrayBuilder {
    builder: StringViewBuilder,
    block: String,
}

impl StringViewArrayBuilder {
    pub fn with_capacity(_item_capacity: usize, data_capacity: usize) -> Self {
        let builder = StringViewBuilder::with_capacity(data_capacity);
        Self {
            builder,
            block: String::new(),
        }
    }

    pub fn write<const CHECK_VALID: bool>(
        &mut self,
        column: &ColumnarValueRef,
        i: usize,
    ) {
        match column {
            ColumnarValueRef::Scalar(s) => {
                self.block.push_str(std::str::from_utf8(s).unwrap());
            }
            ColumnarValueRef::NullableArray(array) => {
                if !CHECK_VALID || array.is_valid(i) {
                    self.block.push_str(
                        std::str::from_utf8(array.value(i).as_bytes()).unwrap(),
                    );
                }
            }
            ColumnarValueRef::NullableLargeStringArray(array) => {
                if !CHECK_VALID || array.is_valid(i) {
                    self.block.push_str(
                        std::str::from_utf8(array.value(i).as_bytes()).unwrap(),
                    );
                }
            }
            ColumnarValueRef::NullableStringViewArray(array) => {
                if !CHECK_VALID || array.is_valid(i) {
                    self.block.push_str(
                        std::str::from_utf8(array.value(i).as_bytes()).unwrap(),
                    );
                }
            }
            ColumnarValueRef::NonNullableArray(array) => {
                self.block
                    .push_str(std::str::from_utf8(array.value(i).as_bytes()).unwrap());
            }
            ColumnarValueRef::NonNullableLargeStringArray(array) => {
                self.block
                    .push_str(std::str::from_utf8(array.value(i).as_bytes()).unwrap());
            }
            ColumnarValueRef::NonNullableStringViewArray(array) => {
                self.block
                    .push_str(std::str::from_utf8(array.value(i).as_bytes()).unwrap());
            }
        }
    }

    pub fn append_offset(&mut self) {
        self.builder.append_value(&self.block);
        self.block = String::new();
    }

    pub fn finish(mut self) -> StringViewArray {
        self.builder.finish()
    }
}

pub struct LargeStringArrayBuilder {
    offsets_buffer: MutableBuffer,
    value_buffer: MutableBuffer,
}

impl LargeStringArrayBuilder {
    pub fn with_capacity(item_capacity: usize, data_capacity: usize) -> Self {
        let capacity = item_capacity
            .checked_add(1)
            .map(|i| i.saturating_mul(size_of::<i64>()))
            .expect("capacity integer overflow");

        let mut offsets_buffer = MutableBuffer::with_capacity(capacity);
        // SAFETY: the first offset value is definitely not going to exceed the bounds.
        unsafe { offsets_buffer.push_unchecked(0_i64) };
        Self {
            offsets_buffer,
            value_buffer: MutableBuffer::with_capacity(data_capacity),
        }
    }

    pub fn write<const CHECK_VALID: bool>(
        &mut self,
        column: &ColumnarValueRef,
        i: usize,
    ) {
        match column {
            ColumnarValueRef::Scalar(s) => {
                self.value_buffer.extend_from_slice(s);
            }
            ColumnarValueRef::NullableArray(array) => {
                if !CHECK_VALID || array.is_valid(i) {
                    self.value_buffer
                        .extend_from_slice(array.value(i).as_bytes());
                }
            }
            ColumnarValueRef::NullableLargeStringArray(array) => {
                if !CHECK_VALID || array.is_valid(i) {
                    self.value_buffer
                        .extend_from_slice(array.value(i).as_bytes());
                }
            }
            ColumnarValueRef::NullableStringViewArray(array) => {
                if !CHECK_VALID || array.is_valid(i) {
                    self.value_buffer
                        .extend_from_slice(array.value(i).as_bytes());
                }
            }
            ColumnarValueRef::NonNullableArray(array) => {
                self.value_buffer
                    .extend_from_slice(array.value(i).as_bytes());
            }
            ColumnarValueRef::NonNullableLargeStringArray(array) => {
                self.value_buffer
                    .extend_from_slice(array.value(i).as_bytes());
            }
            ColumnarValueRef::NonNullableStringViewArray(array) => {
                self.value_buffer
                    .extend_from_slice(array.value(i).as_bytes());
            }
        }
    }

    pub fn append_offset(&mut self) {
        let next_offset: i64 = self
            .value_buffer
            .len()
            .try_into()
            .expect("byte array offset overflow");
        self.offsets_buffer.push(next_offset);
    }

    /// Finalize the builder into a concrete [`LargeStringArray`].
    ///
    /// # Panics
    ///
    /// This method can panic when:
    ///
    /// - the provided `null_buffer` is not the same length as the `offsets_buffer`.
    pub fn finish(self, null_buffer: Option<NullBuffer>) -> LargeStringArray {
        let row_count = self.offsets_buffer.len() / size_of::<i64>() - 1;
        if let Some(ref null_buffer) = null_buffer {
            assert_eq!(
                null_buffer.len(),
                row_count,
                "Null buffer and offsets buffer must be the same length"
            );
        }
        let array_builder = ArrayDataBuilder::new(DataType::LargeUtf8)
            .len(row_count)
            .add_buffer(self.offsets_buffer.into())
            .add_buffer(self.value_buffer.into())
            .nulls(null_buffer);
        // SAFETY: all data that was appended was valid Large UTF8 and the values
        // and offsets were created correctly
        let array_data = unsafe { array_builder.build_unchecked() };
        LargeStringArray::from(array_data)
    }
}

/// Append a new view to the views buffer with the given substr
///
/// # Safety
///
/// original_view must be a valid view (the format described on
/// [`GenericByteViewArray`](arrow::array::GenericByteViewArray).
///
/// # Arguments
/// - views_buffer: The buffer to append the new view to
/// - null_builder: The buffer to append the null value to
/// - original_view: The original view value
/// - substr: The substring to append. Must be a valid substring of the original view
/// - start_offset: The start offset of the substring in the view
pub fn make_and_append_view(
    views_buffer: &mut Vec<u128>,
    null_builder: &mut NullBufferBuilder,
    original_view: &u128,
    substr: &str,
    start_offset: u32,
) {
    let substr_len = substr.len();
    let sub_view = if substr_len > 12 {
        let view = ByteView::from(*original_view);
        make_view(
            substr.as_bytes(),
            view.buffer_index,
            view.offset + start_offset,
        )
    } else {
        // inline value does not need block id or offset
        make_view(substr.as_bytes(), 0, 0)
    };
    views_buffer.push(sub_view);
    null_builder.append_non_null();
}

#[derive(Debug)]
pub enum ColumnarValueRef<'a> {
    Scalar(&'a [u8]),
    NullableArray(&'a StringArray),
    NonNullableArray(&'a StringArray),
    NullableLargeStringArray(&'a LargeStringArray),
    NonNullableLargeStringArray(&'a LargeStringArray),
    NullableStringViewArray(&'a StringViewArray),
    NonNullableStringViewArray(&'a StringViewArray),
}

impl ColumnarValueRef<'_> {
    #[inline]
    pub fn is_valid(&self, i: usize) -> bool {
        match &self {
            Self::Scalar(_)
            | Self::NonNullableArray(_)
            | Self::NonNullableLargeStringArray(_)
            | Self::NonNullableStringViewArray(_) => true,
            Self::NullableArray(array) => array.is_valid(i),
            Self::NullableStringViewArray(array) => array.is_valid(i),
            Self::NullableLargeStringArray(array) => array.is_valid(i),
        }
    }

    #[inline]
    pub fn nulls(&self) -> Option<NullBuffer> {
        match &self {
            Self::Scalar(_)
            | Self::NonNullableArray(_)
            | Self::NonNullableStringViewArray(_)
            | Self::NonNullableLargeStringArray(_) => None,
            Self::NullableArray(array) => array.nulls().cloned(),
            Self::NullableStringViewArray(array) => array.nulls().cloned(),
            Self::NullableLargeStringArray(array) => array.nulls().cloned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "capacity integer overflow")]
    fn test_overflow_string_array_builder() {
        let _builder = StringArrayBuilder::with_capacity(usize::MAX, usize::MAX);
    }

    #[test]
    #[should_panic(expected = "capacity integer overflow")]
    fn test_overflow_large_string_array_builder() {
        let _builder = LargeStringArrayBuilder::with_capacity(usize::MAX, usize::MAX);
    }
}
