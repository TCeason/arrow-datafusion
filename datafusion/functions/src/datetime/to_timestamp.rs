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

use std::any::Any;
use std::sync::Arc;

use crate::datetime::common::*;
use arrow::array::Float64Array;
use arrow::datatypes::DataType::*;
use arrow::datatypes::TimeUnit::{Microsecond, Millisecond, Nanosecond, Second};
use arrow::datatypes::{
    ArrowTimestampType, DataType, TimeUnit, TimestampMicrosecondType,
    TimestampMillisecondType, TimestampNanosecondType, TimestampSecondType,
};
use datafusion_common::format::DEFAULT_CAST_OPTIONS;
use datafusion_common::{exec_err, Result, ScalarType, ScalarValue};
use datafusion_expr::{
    ColumnarValue, Documentation, ScalarUDFImpl, Signature, Volatility,
};
use datafusion_macros::user_doc;

#[user_doc(
    doc_section(label = "Time and Date Functions"),
    description = r#"
Converts a value to a timestamp (`YYYY-MM-DDT00:00:00Z`). Supports strings, integer, unsigned integer, and double types as input. Strings are parsed as RFC3339 (e.g. '2023-07-20T05:44:00') if no [Chrono formats] are provided. Integers, unsigned integers, and doubles are interpreted as seconds since the unix epoch (`1970-01-01T00:00:00Z`). Returns the corresponding timestamp.

Note: `to_timestamp` returns `Timestamp(Nanosecond)`. The supported range for integer input is between `-9223372037` and `9223372036`. Supported range for string input is between `1677-09-21T00:12:44.0` and `2262-04-11T23:47:16.0`. Please use `to_timestamp_seconds` for the input outside of supported bounds.
"#,
    syntax_example = "to_timestamp(expression[, ..., format_n])",
    sql_example = r#"```sql
> select to_timestamp('2023-01-31T09:26:56.123456789-05:00');
+-----------------------------------------------------------+
| to_timestamp(Utf8("2023-01-31T09:26:56.123456789-05:00")) |
+-----------------------------------------------------------+
| 2023-01-31T14:26:56.123456789                             |
+-----------------------------------------------------------+
> select to_timestamp('03:59:00.123456789 05-17-2023', '%c', '%+', '%H:%M:%S%.f %m-%d-%Y');
+--------------------------------------------------------------------------------------------------------+
| to_timestamp(Utf8("03:59:00.123456789 05-17-2023"),Utf8("%c"),Utf8("%+"),Utf8("%H:%M:%S%.f %m-%d-%Y")) |
+--------------------------------------------------------------------------------------------------------+
| 2023-05-17T03:59:00.123456789                                                                          |
+--------------------------------------------------------------------------------------------------------+
```
Additional examples can be found [here](https://github.com/apache/datafusion/blob/main/datafusion-examples/examples/to_timestamp.rs)
"#,
    argument(
        name = "expression",
        description = "Expression to operate on. Can be a constant, column, or function, and any combination of arithmetic operators."
    ),
    argument(
        name = "format_n",
        description = "Optional [Chrono format](https://docs.rs/chrono/latest/chrono/format/strftime/index.html) strings to use to parse the expression. Formats will be tried in the order they appear with the first successful one being returned. If none of the formats successfully parse the expression an error will be returned."
    )
)]
#[derive(Debug)]
pub struct ToTimestampFunc {
    signature: Signature,
}

#[user_doc(
    doc_section(label = "Time and Date Functions"),
    description = "Converts a value to a timestamp (`YYYY-MM-DDT00:00:00.000Z`). Supports strings, integer, and unsigned integer types as input. Strings are parsed as RFC3339 (e.g. '2023-07-20T05:44:00') if no [Chrono format](https://docs.rs/chrono/latest/chrono/format/strftime/index.html)s are provided. Integers and unsigned integers are interpreted as seconds since the unix epoch (`1970-01-01T00:00:00Z`). Returns the corresponding timestamp.",
    syntax_example = "to_timestamp_seconds(expression[, ..., format_n])",
    sql_example = r#"```sql
> select to_timestamp_seconds('2023-01-31T09:26:56.123456789-05:00');
+-------------------------------------------------------------------+
| to_timestamp_seconds(Utf8("2023-01-31T09:26:56.123456789-05:00")) |
+-------------------------------------------------------------------+
| 2023-01-31T14:26:56                                               |
+-------------------------------------------------------------------+
> select to_timestamp_seconds('03:59:00.123456789 05-17-2023', '%c', '%+', '%H:%M:%S%.f %m-%d-%Y');
+----------------------------------------------------------------------------------------------------------------+
| to_timestamp_seconds(Utf8("03:59:00.123456789 05-17-2023"),Utf8("%c"),Utf8("%+"),Utf8("%H:%M:%S%.f %m-%d-%Y")) |
+----------------------------------------------------------------------------------------------------------------+
| 2023-05-17T03:59:00                                                                                            |
+----------------------------------------------------------------------------------------------------------------+
```
Additional examples can be found [here](https://github.com/apache/datafusion/blob/main/datafusion-examples/examples/to_timestamp.rs)
"#,
    argument(
        name = "expression",
        description = "Expression to operate on. Can be a constant, column, or function, and any combination of arithmetic operators."
    ),
    argument(
        name = "format_n",
        description = "Optional [Chrono format](https://docs.rs/chrono/latest/chrono/format/strftime/index.html) strings to use to parse the expression. Formats will be tried in the order they appear with the first successful one being returned. If none of the formats successfully parse the expression an error will be returned."
    )
)]
#[derive(Debug)]
pub struct ToTimestampSecondsFunc {
    signature: Signature,
}

#[user_doc(
    doc_section(label = "Time and Date Functions"),
    description = "Converts a value to a timestamp (`YYYY-MM-DDT00:00:00.000Z`). Supports strings, integer, and unsigned integer types as input. Strings are parsed as RFC3339 (e.g. '2023-07-20T05:44:00') if no [Chrono formats](https://docs.rs/chrono/latest/chrono/format/strftime/index.html) are provided. Integers and unsigned integers are interpreted as milliseconds since the unix epoch (`1970-01-01T00:00:00Z`). Returns the corresponding timestamp.",
    syntax_example = "to_timestamp_millis(expression[, ..., format_n])",
    sql_example = r#"```sql
> select to_timestamp_millis('2023-01-31T09:26:56.123456789-05:00');
+------------------------------------------------------------------+
| to_timestamp_millis(Utf8("2023-01-31T09:26:56.123456789-05:00")) |
+------------------------------------------------------------------+
| 2023-01-31T14:26:56.123                                          |
+------------------------------------------------------------------+
> select to_timestamp_millis('03:59:00.123456789 05-17-2023', '%c', '%+', '%H:%M:%S%.f %m-%d-%Y');
+---------------------------------------------------------------------------------------------------------------+
| to_timestamp_millis(Utf8("03:59:00.123456789 05-17-2023"),Utf8("%c"),Utf8("%+"),Utf8("%H:%M:%S%.f %m-%d-%Y")) |
+---------------------------------------------------------------------------------------------------------------+
| 2023-05-17T03:59:00.123                                                                                       |
+---------------------------------------------------------------------------------------------------------------+
```
Additional examples can be found [here](https://github.com/apache/datafusion/blob/main/datafusion-examples/examples/to_timestamp.rs)
"#,
    argument(
        name = "expression",
        description = "Expression to operate on. Can be a constant, column, or function, and any combination of arithmetic operators."
    ),
    argument(
        name = "format_n",
        description = "Optional [Chrono format](https://docs.rs/chrono/latest/chrono/format/strftime/index.html) strings to use to parse the expression. Formats will be tried in the order they appear with the first successful one being returned. If none of the formats successfully parse the expression an error will be returned."
    )
)]
#[derive(Debug)]
pub struct ToTimestampMillisFunc {
    signature: Signature,
}

#[user_doc(
    doc_section(label = "Time and Date Functions"),
    description = "Converts a value to a timestamp (`YYYY-MM-DDT00:00:00.000000Z`). Supports strings, integer, and unsigned integer types as input. Strings are parsed as RFC3339 (e.g. '2023-07-20T05:44:00') if no [Chrono format](https://docs.rs/chrono/latest/chrono/format/strftime/index.html)s are provided. Integers and unsigned integers are interpreted as microseconds since the unix epoch (`1970-01-01T00:00:00Z`) Returns the corresponding timestamp.",
    syntax_example = "to_timestamp_micros(expression[, ..., format_n])",
    sql_example = r#"```sql
> select to_timestamp_micros('2023-01-31T09:26:56.123456789-05:00');
+------------------------------------------------------------------+
| to_timestamp_micros(Utf8("2023-01-31T09:26:56.123456789-05:00")) |
+------------------------------------------------------------------+
| 2023-01-31T14:26:56.123456                                       |
+------------------------------------------------------------------+
> select to_timestamp_micros('03:59:00.123456789 05-17-2023', '%c', '%+', '%H:%M:%S%.f %m-%d-%Y');
+---------------------------------------------------------------------------------------------------------------+
| to_timestamp_micros(Utf8("03:59:00.123456789 05-17-2023"),Utf8("%c"),Utf8("%+"),Utf8("%H:%M:%S%.f %m-%d-%Y")) |
+---------------------------------------------------------------------------------------------------------------+
| 2023-05-17T03:59:00.123456                                                                                    |
+---------------------------------------------------------------------------------------------------------------+
```
Additional examples can be found [here](https://github.com/apache/datafusion/blob/main/datafusion-examples/examples/to_timestamp.rs)
"#,
    argument(
        name = "expression",
        description = "Expression to operate on. Can be a constant, column, or function, and any combination of arithmetic operators."
    ),
    argument(
        name = "format_n",
        description = "Optional [Chrono format](https://docs.rs/chrono/latest/chrono/format/strftime/index.html) strings to use to parse the expression. Formats will be tried in the order they appear with the first successful one being returned. If none of the formats successfully parse the expression an error will be returned."
    )
)]
#[derive(Debug)]
pub struct ToTimestampMicrosFunc {
    signature: Signature,
}

#[user_doc(
    doc_section(label = "Time and Date Functions"),
    description = "Converts a value to a timestamp (`YYYY-MM-DDT00:00:00.000000000Z`). Supports strings, integer, and unsigned integer types as input. Strings are parsed as RFC3339 (e.g. '2023-07-20T05:44:00') if no [Chrono format](https://docs.rs/chrono/latest/chrono/format/strftime/index.html)s are provided. Integers and unsigned integers are interpreted as nanoseconds since the unix epoch (`1970-01-01T00:00:00Z`). Returns the corresponding timestamp.",
    syntax_example = "to_timestamp_nanos(expression[, ..., format_n])",
    sql_example = r#"```sql
> select to_timestamp_nanos('2023-01-31T09:26:56.123456789-05:00');
+-----------------------------------------------------------------+
| to_timestamp_nanos(Utf8("2023-01-31T09:26:56.123456789-05:00")) |
+-----------------------------------------------------------------+
| 2023-01-31T14:26:56.123456789                                   |
+-----------------------------------------------------------------+
> select to_timestamp_nanos('03:59:00.123456789 05-17-2023', '%c', '%+', '%H:%M:%S%.f %m-%d-%Y');
+--------------------------------------------------------------------------------------------------------------+
| to_timestamp_nanos(Utf8("03:59:00.123456789 05-17-2023"),Utf8("%c"),Utf8("%+"),Utf8("%H:%M:%S%.f %m-%d-%Y")) |
+--------------------------------------------------------------------------------------------------------------+
| 2023-05-17T03:59:00.123456789                                                                                |
+---------------------------------------------------------------------------------------------------------------+
```
Additional examples can be found [here](https://github.com/apache/datafusion/blob/main/datafusion-examples/examples/to_timestamp.rs)
"#,
    argument(
        name = "expression",
        description = "Expression to operate on. Can be a constant, column, or function, and any combination of arithmetic operators."
    ),
    argument(
        name = "format_n",
        description = "Optional [Chrono format](https://docs.rs/chrono/latest/chrono/format/strftime/index.html) strings to use to parse the expression. Formats will be tried in the order they appear with the first successful one being returned. If none of the formats successfully parse the expression an error will be returned."
    )
)]
#[derive(Debug)]
pub struct ToTimestampNanosFunc {
    signature: Signature,
}

impl Default for ToTimestampFunc {
    fn default() -> Self {
        Self::new()
    }
}

impl ToTimestampFunc {
    pub fn new() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Immutable),
        }
    }
}

impl Default for ToTimestampSecondsFunc {
    fn default() -> Self {
        Self::new()
    }
}

impl ToTimestampSecondsFunc {
    pub fn new() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Immutable),
        }
    }
}

impl Default for ToTimestampMillisFunc {
    fn default() -> Self {
        Self::new()
    }
}

impl ToTimestampMillisFunc {
    pub fn new() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Immutable),
        }
    }
}

impl Default for ToTimestampMicrosFunc {
    fn default() -> Self {
        Self::new()
    }
}

impl ToTimestampMicrosFunc {
    pub fn new() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Immutable),
        }
    }
}

impl Default for ToTimestampNanosFunc {
    fn default() -> Self {
        Self::new()
    }
}

impl ToTimestampNanosFunc {
    pub fn new() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Immutable),
        }
    }
}

/// to_timestamp SQL function
///
/// Note: `to_timestamp` returns `Timestamp(Nanosecond)` though its arguments are interpreted as **seconds**.
/// The supported range for integer input is between `-9223372037` and `9223372036`.
/// Supported range for string input is between `1677-09-21T00:12:44.0` and `2262-04-11T23:47:16.0`.
/// Please use `to_timestamp_seconds` for the input outside of supported bounds.
impl ScalarUDFImpl for ToTimestampFunc {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn name(&self) -> &str {
        "to_timestamp"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, arg_types: &[DataType]) -> Result<DataType> {
        Ok(return_type_for(&arg_types[0], Nanosecond))
    }

    fn invoke_with_args(
        &self,
        args: datafusion_expr::ScalarFunctionArgs,
    ) -> Result<ColumnarValue> {
        let args = args.args;
        if args.is_empty() {
            return exec_err!(
                "to_timestamp function requires 1 or more arguments, got {}",
                args.len()
            );
        }

        // validate that any args after the first one are Utf8
        if args.len() > 1 {
            validate_data_types(&args, "to_timestamp")?;
        }

        match args[0].data_type() {
            Int32 | Int64 => args[0]
                .cast_to(&Timestamp(Second, None), None)?
                .cast_to(&Timestamp(Nanosecond, None), None),
            Null | Timestamp(_, None) => {
                args[0].cast_to(&Timestamp(Nanosecond, None), None)
            }
            Float64 => {
                let rescaled = arrow::compute::kernels::numeric::mul(
                    &args[0].to_array(1)?,
                    &arrow::array::Scalar::new(Float64Array::from(vec![
                        1_000_000_000f64,
                    ])),
                )?;
                Ok(ColumnarValue::Array(arrow::compute::cast_with_options(
                    &rescaled,
                    &Timestamp(Nanosecond, None),
                    &DEFAULT_CAST_OPTIONS,
                )?))
            }
            Timestamp(_, Some(tz)) => {
                args[0].cast_to(&Timestamp(Nanosecond, Some(tz)), None)
            }
            Utf8View | LargeUtf8 | Utf8 => {
                to_timestamp_impl::<TimestampNanosecondType>(&args, "to_timestamp")
            }
            Decimal128(_, _) => {
                match &args[0] {
                    ColumnarValue::Scalar(ScalarValue::Decimal128(
                        Some(value),
                        _,
                        scale,
                    )) => {
                        // Convert decimal to seconds and nanoseconds
                        let scale_factor = 10_i128.pow(*scale as u32);
                        let seconds = value / scale_factor;
                        let fraction = value % scale_factor;

                        let nanos = (fraction * 1_000_000_000) / scale_factor;

                        let timestamp_nanos = seconds * 1_000_000_000 + nanos;

                        Ok(ColumnarValue::Scalar(ScalarValue::TimestampNanosecond(
                            Some(timestamp_nanos as i64),
                            None,
                        )))
                    }
                    _ => exec_err!("Invalid decimal value"),
                }
            }
            other => {
                exec_err!(
                    "Unsupported data type {:?} for function to_timestamp",
                    other
                )
            }
        }
    }
    fn documentation(&self) -> Option<&Documentation> {
        self.doc()
    }
}

impl ScalarUDFImpl for ToTimestampSecondsFunc {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn name(&self) -> &str {
        "to_timestamp_seconds"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, arg_types: &[DataType]) -> Result<DataType> {
        Ok(return_type_for(&arg_types[0], Second))
    }

    fn invoke_with_args(
        &self,
        args: datafusion_expr::ScalarFunctionArgs,
    ) -> Result<ColumnarValue> {
        let args = args.args;
        if args.is_empty() {
            return exec_err!(
                "to_timestamp_seconds function requires 1 or more arguments, got {}",
                args.len()
            );
        }

        // validate that any args after the first one are Utf8
        if args.len() > 1 {
            validate_data_types(&args, "to_timestamp")?;
        }

        match args[0].data_type() {
            Null | Int32 | Int64 | Timestamp(_, None) | Decimal128(_, _) => {
                args[0].cast_to(&Timestamp(Second, None), None)
            }
            Timestamp(_, Some(tz)) => args[0].cast_to(&Timestamp(Second, Some(tz)), None),
            Utf8View | LargeUtf8 | Utf8 => {
                to_timestamp_impl::<TimestampSecondType>(&args, "to_timestamp_seconds")
            }
            other => {
                exec_err!(
                    "Unsupported data type {:?} for function to_timestamp_seconds",
                    other
                )
            }
        }
    }
    fn documentation(&self) -> Option<&Documentation> {
        self.doc()
    }
}

impl ScalarUDFImpl for ToTimestampMillisFunc {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn name(&self) -> &str {
        "to_timestamp_millis"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, arg_types: &[DataType]) -> Result<DataType> {
        Ok(return_type_for(&arg_types[0], Millisecond))
    }

    fn invoke_with_args(
        &self,
        args: datafusion_expr::ScalarFunctionArgs,
    ) -> Result<ColumnarValue> {
        let args = args.args;
        if args.is_empty() {
            return exec_err!(
                "to_timestamp_millis function requires 1 or more arguments, got {}",
                args.len()
            );
        }

        // validate that any args after the first one are Utf8
        if args.len() > 1 {
            validate_data_types(&args, "to_timestamp")?;
        }

        match args[0].data_type() {
            Null | Int32 | Int64 | Timestamp(_, None) => {
                args[0].cast_to(&Timestamp(Millisecond, None), None)
            }
            Timestamp(_, Some(tz)) => {
                args[0].cast_to(&Timestamp(Millisecond, Some(tz)), None)
            }
            Utf8View | LargeUtf8 | Utf8 => to_timestamp_impl::<TimestampMillisecondType>(
                &args,
                "to_timestamp_millis",
            ),
            other => {
                exec_err!(
                    "Unsupported data type {:?} for function to_timestamp_millis",
                    other
                )
            }
        }
    }
    fn documentation(&self) -> Option<&Documentation> {
        self.doc()
    }
}

impl ScalarUDFImpl for ToTimestampMicrosFunc {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn name(&self) -> &str {
        "to_timestamp_micros"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, arg_types: &[DataType]) -> Result<DataType> {
        Ok(return_type_for(&arg_types[0], Microsecond))
    }

    fn invoke_with_args(
        &self,
        args: datafusion_expr::ScalarFunctionArgs,
    ) -> Result<ColumnarValue> {
        let args = args.args;
        if args.is_empty() {
            return exec_err!(
                "to_timestamp_micros function requires 1 or more arguments, got {}",
                args.len()
            );
        }

        // validate that any args after the first one are Utf8
        if args.len() > 1 {
            validate_data_types(&args, "to_timestamp")?;
        }

        match args[0].data_type() {
            Null | Int32 | Int64 | Timestamp(_, None) => {
                args[0].cast_to(&Timestamp(Microsecond, None), None)
            }
            Timestamp(_, Some(tz)) => {
                args[0].cast_to(&Timestamp(Microsecond, Some(tz)), None)
            }
            Utf8View | LargeUtf8 | Utf8 => to_timestamp_impl::<TimestampMicrosecondType>(
                &args,
                "to_timestamp_micros",
            ),
            other => {
                exec_err!(
                    "Unsupported data type {:?} for function to_timestamp_micros",
                    other
                )
            }
        }
    }
    fn documentation(&self) -> Option<&Documentation> {
        self.doc()
    }
}

impl ScalarUDFImpl for ToTimestampNanosFunc {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn name(&self) -> &str {
        "to_timestamp_nanos"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, arg_types: &[DataType]) -> Result<DataType> {
        Ok(return_type_for(&arg_types[0], Nanosecond))
    }

    fn invoke_with_args(
        &self,
        args: datafusion_expr::ScalarFunctionArgs,
    ) -> Result<ColumnarValue> {
        let args = args.args;
        if args.is_empty() {
            return exec_err!(
                "to_timestamp_nanos function requires 1 or more arguments, got {}",
                args.len()
            );
        }

        // validate that any args after the first one are Utf8
        if args.len() > 1 {
            validate_data_types(&args, "to_timestamp")?;
        }

        match args[0].data_type() {
            Null | Int32 | Int64 | Timestamp(_, None) => {
                args[0].cast_to(&Timestamp(Nanosecond, None), None)
            }
            Timestamp(_, Some(tz)) => {
                args[0].cast_to(&Timestamp(Nanosecond, Some(tz)), None)
            }
            Utf8View | LargeUtf8 | Utf8 => {
                to_timestamp_impl::<TimestampNanosecondType>(&args, "to_timestamp_nanos")
            }
            other => {
                exec_err!(
                    "Unsupported data type {:?} for function to_timestamp_nanos",
                    other
                )
            }
        }
    }
    fn documentation(&self) -> Option<&Documentation> {
        self.doc()
    }
}

/// Returns the return type for the to_timestamp_* function, preserving
/// the timezone if it exists.
fn return_type_for(arg: &DataType, unit: TimeUnit) -> DataType {
    match arg {
        Timestamp(_, Some(tz)) => Timestamp(unit, Some(Arc::clone(tz))),
        _ => Timestamp(unit, None),
    }
}

fn to_timestamp_impl<T: ArrowTimestampType + ScalarType<i64>>(
    args: &[ColumnarValue],
    name: &str,
) -> Result<ColumnarValue> {
    let factor = match T::UNIT {
        Second => 1_000_000_000,
        Millisecond => 1_000_000,
        Microsecond => 1_000,
        Nanosecond => 1,
    };

    match args.len() {
        1 => handle::<T, _, T>(
            args,
            |s| string_to_timestamp_nanos_shim(s).map(|n| n / factor),
            name,
        ),
        n if n >= 2 => handle_multiple::<T, _, T, _>(
            args,
            string_to_timestamp_nanos_formatted,
            |n| n / factor,
            name,
        ),
        _ => exec_err!("Unsupported 0 argument count for function {name}"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::types::Int64Type;
    use arrow::array::{
        Array, PrimitiveArray, TimestampMicrosecondArray, TimestampMillisecondArray,
        TimestampNanosecondArray, TimestampSecondArray,
    };
    use arrow::array::{ArrayRef, Int64Array, StringBuilder};
    use arrow::datatypes::{Field, TimeUnit};
    use chrono::Utc;
    use datafusion_common::{assert_contains, DataFusionError, ScalarValue};
    use datafusion_expr::ScalarFunctionImplementation;

    use super::*;

    fn to_timestamp(args: &[ColumnarValue]) -> Result<ColumnarValue> {
        to_timestamp_impl::<TimestampNanosecondType>(args, "to_timestamp")
    }

    /// to_timestamp_millis SQL function
    fn to_timestamp_millis(args: &[ColumnarValue]) -> Result<ColumnarValue> {
        to_timestamp_impl::<TimestampMillisecondType>(args, "to_timestamp_millis")
    }

    /// to_timestamp_micros SQL function
    fn to_timestamp_micros(args: &[ColumnarValue]) -> Result<ColumnarValue> {
        to_timestamp_impl::<TimestampMicrosecondType>(args, "to_timestamp_micros")
    }

    /// to_timestamp_nanos SQL function
    fn to_timestamp_nanos(args: &[ColumnarValue]) -> Result<ColumnarValue> {
        to_timestamp_impl::<TimestampNanosecondType>(args, "to_timestamp_nanos")
    }

    /// to_timestamp_seconds SQL function
    fn to_timestamp_seconds(args: &[ColumnarValue]) -> Result<ColumnarValue> {
        to_timestamp_impl::<TimestampSecondType>(args, "to_timestamp_seconds")
    }

    #[test]
    fn to_timestamp_arrays_and_nulls() -> Result<()> {
        // ensure that arrow array implementation is wired up and handles nulls correctly

        let mut string_builder = StringBuilder::with_capacity(2, 1024);
        let mut ts_builder = TimestampNanosecondArray::builder(2);

        string_builder.append_value("2020-09-08T13:42:29.190855");
        ts_builder.append_value(1599572549190855000);

        string_builder.append_null();
        ts_builder.append_null();
        let expected_timestamps = &ts_builder.finish() as &dyn Array;

        let string_array =
            ColumnarValue::Array(Arc::new(string_builder.finish()) as ArrayRef);
        let parsed_timestamps = to_timestamp(&[string_array])
            .expect("that to_timestamp parsed values without error");
        if let ColumnarValue::Array(parsed_array) = parsed_timestamps {
            assert_eq!(parsed_array.len(), 2);
            assert_eq!(expected_timestamps, parsed_array.as_ref());
        } else {
            panic!("Expected a columnar array")
        }
        Ok(())
    }

    #[test]
    fn to_timestamp_with_formats_arrays_and_nulls() -> Result<()> {
        // ensure that arrow array implementation is wired up and handles nulls correctly

        let mut date_string_builder = StringBuilder::with_capacity(2, 1024);
        let mut format1_builder = StringBuilder::with_capacity(2, 1024);
        let mut format2_builder = StringBuilder::with_capacity(2, 1024);
        let mut format3_builder = StringBuilder::with_capacity(2, 1024);
        let mut ts_builder = TimestampNanosecondArray::builder(2);

        date_string_builder.append_null();
        format1_builder.append_null();
        format2_builder.append_null();
        format3_builder.append_null();
        ts_builder.append_null();

        date_string_builder.append_value("2020-09-08T13:42:29.19085Z");
        format1_builder.append_value("%s");
        format2_builder.append_value("%c");
        format3_builder.append_value("%+");
        ts_builder.append_value(1599572549190850000);

        let expected_timestamps = &ts_builder.finish() as &dyn Array;

        let string_array = [
            ColumnarValue::Array(Arc::new(date_string_builder.finish()) as ArrayRef),
            ColumnarValue::Array(Arc::new(format1_builder.finish()) as ArrayRef),
            ColumnarValue::Array(Arc::new(format2_builder.finish()) as ArrayRef),
            ColumnarValue::Array(Arc::new(format3_builder.finish()) as ArrayRef),
        ];
        let parsed_timestamps = to_timestamp(&string_array)
            .expect("that to_timestamp with format args parsed values without error");
        if let ColumnarValue::Array(parsed_array) = parsed_timestamps {
            assert_eq!(parsed_array.len(), 2);
            assert_eq!(expected_timestamps, parsed_array.as_ref());
        } else {
            panic!("Expected a columnar array")
        }
        Ok(())
    }

    #[test]
    fn to_timestamp_invalid_input_type() -> Result<()> {
        // pass the wrong type of input array to to_timestamp and test
        // that we get an error.

        let mut builder = Int64Array::builder(1);
        builder.append_value(1);
        let int64array = ColumnarValue::Array(Arc::new(builder.finish()));

        let expected_err =
            "Execution error: Unsupported data type Int64 for function to_timestamp";
        match to_timestamp(&[int64array]) {
            Ok(_) => panic!("Expected error but got success"),
            Err(e) => {
                assert!(
                    e.to_string().contains(expected_err),
                    "Can not find expected error '{expected_err}'. Actual error '{e}'"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn to_timestamp_with_formats_invalid_input_type() -> Result<()> {
        // pass the wrong type of input array to to_timestamp and test
        // that we get an error.

        let mut builder = Int64Array::builder(1);
        builder.append_value(1);
        let int64array = [
            ColumnarValue::Array(Arc::new(builder.finish())),
            ColumnarValue::Array(Arc::new(builder.finish())),
        ];

        let expected_err =
            "Execution error: Unsupported data type Int64 for function to_timestamp";
        match to_timestamp(&int64array) {
            Ok(_) => panic!("Expected error but got success"),
            Err(e) => {
                assert!(
                    e.to_string().contains(expected_err),
                    "Can not find expected error '{expected_err}'. Actual error '{e}'"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn to_timestamp_with_unparseable_data() -> Result<()> {
        let mut date_string_builder = StringBuilder::with_capacity(2, 1024);

        date_string_builder.append_null();

        date_string_builder.append_value("2020-09-08 - 13:42:29.19085Z");

        let string_array =
            ColumnarValue::Array(Arc::new(date_string_builder.finish()) as ArrayRef);

        let expected_err =
            "Arrow error: Parser error: Error parsing timestamp from '2020-09-08 - 13:42:29.19085Z': error parsing time";
        match to_timestamp(&[string_array]) {
            Ok(_) => panic!("Expected error but got success"),
            Err(e) => {
                assert!(
                    e.to_string().contains(expected_err),
                    "Can not find expected error '{expected_err}'. Actual error '{e}'"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn to_timestamp_with_invalid_tz() -> Result<()> {
        let mut date_string_builder = StringBuilder::with_capacity(2, 1024);

        date_string_builder.append_null();

        date_string_builder.append_value("2020-09-08T13:42:29ZZ");

        let string_array =
            ColumnarValue::Array(Arc::new(date_string_builder.finish()) as ArrayRef);

        let expected_err =
            "Arrow error: Parser error: Invalid timezone \"ZZ\": failed to parse timezone";
        match to_timestamp(&[string_array]) {
            Ok(_) => panic!("Expected error but got success"),
            Err(e) => {
                assert!(
                    e.to_string().contains(expected_err),
                    "Can not find expected error '{expected_err}'. Actual error '{e}'"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn to_timestamp_with_no_matching_formats() -> Result<()> {
        let mut date_string_builder = StringBuilder::with_capacity(2, 1024);
        let mut format1_builder = StringBuilder::with_capacity(2, 1024);
        let mut format2_builder = StringBuilder::with_capacity(2, 1024);
        let mut format3_builder = StringBuilder::with_capacity(2, 1024);

        date_string_builder.append_null();
        format1_builder.append_null();
        format2_builder.append_null();
        format3_builder.append_null();

        date_string_builder.append_value("2020-09-08T13:42:29.19085Z");
        format1_builder.append_value("%s");
        format2_builder.append_value("%c");
        format3_builder.append_value("%H:%M:%S");

        let string_array = [
            ColumnarValue::Array(Arc::new(date_string_builder.finish()) as ArrayRef),
            ColumnarValue::Array(Arc::new(format1_builder.finish()) as ArrayRef),
            ColumnarValue::Array(Arc::new(format2_builder.finish()) as ArrayRef),
            ColumnarValue::Array(Arc::new(format3_builder.finish()) as ArrayRef),
        ];

        let expected_err =
            "Execution error: Error parsing timestamp from '2020-09-08T13:42:29.19085Z' using format '%H:%M:%S': input contains invalid characters";
        match to_timestamp(&string_array) {
            Ok(_) => panic!("Expected error but got success"),
            Err(e) => {
                assert!(
                    e.to_string().contains(expected_err),
                    "Can not find expected error '{expected_err}'. Actual error '{e}'"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn string_to_timestamp_formatted() {
        // Explicit timezone
        assert_eq!(
            1599572549190855000,
            parse_timestamp_formatted("2020-09-08T13:42:29.190855+00:00", "%+").unwrap()
        );
        assert_eq!(
            1599572549190855000,
            parse_timestamp_formatted("2020-09-08T13:42:29.190855Z", "%+").unwrap()
        );
        assert_eq!(
            1599572549000000000,
            parse_timestamp_formatted("2020-09-08T13:42:29Z", "%+").unwrap()
        ); // no fractional part
        assert_eq!(
            1599590549190855000,
            parse_timestamp_formatted("2020-09-08T13:42:29.190855-05:00", "%+").unwrap()
        );
        assert_eq!(
            1599590549000000000,
            parse_timestamp_formatted("1599590549", "%s").unwrap()
        );
        assert_eq!(
            1599572549000000000,
            parse_timestamp_formatted("09-08-2020 13/42/29", "%m-%d-%Y %H/%M/%S")
                .unwrap()
        );
        assert_eq!(
            1642896000000000000,
            parse_timestamp_formatted("2022-01-23", "%Y-%m-%d").unwrap()
        );
    }

    fn parse_timestamp_formatted(s: &str, format: &str) -> Result<i64, DataFusionError> {
        let result = string_to_timestamp_nanos_formatted(s, format);
        if let Err(e) = &result {
            eprintln!("Error parsing timestamp '{s}' using format '{format}': {e:?}");
        }
        result
    }

    #[test]
    fn string_to_timestamp_formatted_invalid() {
        // Test parsing invalid formats
        let cases = [
            ("", "%Y%m%d %H%M%S", "premature end of input"),
            ("SS", "%c", "premature end of input"),
            ("Wed, 18 Feb 2015 23:16:09 GMT", "", "trailing input"),
            (
                "Wed, 18 Feb 2015 23:16:09 GMT",
                "%XX",
                "input contains invalid characters",
            ),
            (
                "Wed, 18 Feb 2015 23:16:09 GMT",
                "%Y%m%d %H%M%S",
                "input contains invalid characters",
            ),
        ];

        for (s, f, ctx) in cases {
            let expected = format!("Execution error: Error parsing timestamp from '{s}' using format '{f}': {ctx}");
            let actual = string_to_datetime_formatted(&Utc, s, f)
                .unwrap_err()
                .to_string();
            assert_eq!(actual, expected)
        }
    }

    #[test]
    fn string_to_timestamp_invalid_arguments() {
        // Test parsing invalid formats
        let cases = [
            ("", "%Y%m%d %H%M%S", "premature end of input"),
            ("SS", "%c", "premature end of input"),
            ("Wed, 18 Feb 2015 23:16:09 GMT", "", "trailing input"),
            (
                "Wed, 18 Feb 2015 23:16:09 GMT",
                "%XX",
                "input contains invalid characters",
            ),
            (
                "Wed, 18 Feb 2015 23:16:09 GMT",
                "%Y%m%d %H%M%S",
                "input contains invalid characters",
            ),
        ];

        for (s, f, ctx) in cases {
            let expected = format!("Execution error: Error parsing timestamp from '{s}' using format '{f}': {ctx}");
            let actual = string_to_datetime_formatted(&Utc, s, f)
                .unwrap_err()
                .to_string();
            assert_eq!(actual, expected)
        }
    }

    #[test]
    fn test_tz() {
        let udfs: Vec<Box<dyn ScalarUDFImpl>> = vec![
            Box::new(ToTimestampFunc::new()),
            Box::new(ToTimestampSecondsFunc::new()),
            Box::new(ToTimestampMillisFunc::new()),
            Box::new(ToTimestampNanosFunc::new()),
            Box::new(ToTimestampSecondsFunc::new()),
        ];

        let mut nanos_builder = TimestampNanosecondArray::builder(2);
        let mut millis_builder = TimestampMillisecondArray::builder(2);
        let mut micros_builder = TimestampMicrosecondArray::builder(2);
        let mut sec_builder = TimestampSecondArray::builder(2);

        nanos_builder.append_value(1599572549190850000);
        millis_builder.append_value(1599572549190);
        micros_builder.append_value(1599572549190850);
        sec_builder.append_value(1599572549);

        let nanos_timestamps =
            Arc::new(nanos_builder.finish().with_timezone("UTC")) as ArrayRef;
        let millis_timestamps =
            Arc::new(millis_builder.finish().with_timezone("UTC")) as ArrayRef;
        let micros_timestamps =
            Arc::new(micros_builder.finish().with_timezone("UTC")) as ArrayRef;
        let sec_timestamps =
            Arc::new(sec_builder.finish().with_timezone("UTC")) as ArrayRef;

        let arrays = &[
            ColumnarValue::Array(Arc::clone(&nanos_timestamps)),
            ColumnarValue::Array(Arc::clone(&millis_timestamps)),
            ColumnarValue::Array(Arc::clone(&micros_timestamps)),
            ColumnarValue::Array(Arc::clone(&sec_timestamps)),
        ];

        for udf in &udfs {
            for array in arrays {
                let rt = udf.return_type(&[array.data_type()]).unwrap();
                let arg_field = Field::new("arg", array.data_type().clone(), true).into();
                assert!(matches!(rt, Timestamp(_, Some(_))));
                let args = datafusion_expr::ScalarFunctionArgs {
                    args: vec![array.clone()],
                    arg_fields: vec![arg_field],
                    number_rows: 4,
                    return_field: Field::new("f", rt, true).into(),
                };
                let res = udf
                    .invoke_with_args(args)
                    .expect("that to_timestamp parsed values without error");
                let array = match res {
                    ColumnarValue::Array(res) => res,
                    _ => panic!("Expected a columnar array"),
                };
                let ty = array.data_type();
                assert!(matches!(ty, Timestamp(_, Some(_))));
            }
        }

        let mut nanos_builder = TimestampNanosecondArray::builder(2);
        let mut millis_builder = TimestampMillisecondArray::builder(2);
        let mut micros_builder = TimestampMicrosecondArray::builder(2);
        let mut sec_builder = TimestampSecondArray::builder(2);
        let mut i64_builder = Int64Array::builder(2);

        nanos_builder.append_value(1599572549190850000);
        millis_builder.append_value(1599572549190);
        micros_builder.append_value(1599572549190850);
        sec_builder.append_value(1599572549);
        i64_builder.append_value(1599572549);

        let nanos_timestamps = Arc::new(nanos_builder.finish()) as ArrayRef;
        let millis_timestamps = Arc::new(millis_builder.finish()) as ArrayRef;
        let micros_timestamps = Arc::new(micros_builder.finish()) as ArrayRef;
        let sec_timestamps = Arc::new(sec_builder.finish()) as ArrayRef;
        let i64_timestamps = Arc::new(i64_builder.finish()) as ArrayRef;

        let arrays = &[
            ColumnarValue::Array(Arc::clone(&nanos_timestamps)),
            ColumnarValue::Array(Arc::clone(&millis_timestamps)),
            ColumnarValue::Array(Arc::clone(&micros_timestamps)),
            ColumnarValue::Array(Arc::clone(&sec_timestamps)),
            ColumnarValue::Array(Arc::clone(&i64_timestamps)),
        ];

        for udf in &udfs {
            for array in arrays {
                let rt = udf.return_type(&[array.data_type()]).unwrap();
                assert!(matches!(rt, Timestamp(_, None)));
                let arg_field = Field::new("arg", array.data_type().clone(), true).into();
                let args = datafusion_expr::ScalarFunctionArgs {
                    args: vec![array.clone()],
                    arg_fields: vec![arg_field],
                    number_rows: 5,
                    return_field: Field::new("f", rt, true).into(),
                };
                let res = udf
                    .invoke_with_args(args)
                    .expect("that to_timestamp parsed values without error");
                let array = match res {
                    ColumnarValue::Array(res) => res,
                    _ => panic!("Expected a columnar array"),
                };
                let ty = array.data_type();
                assert!(matches!(ty, Timestamp(_, None)));
            }
        }
    }

    #[test]
    fn test_to_timestamp_arg_validation() {
        let mut date_string_builder = StringBuilder::with_capacity(2, 1024);
        date_string_builder.append_value("2020-09-08T13:42:29.19085Z");

        let data = date_string_builder.finish();

        let funcs: Vec<(ScalarFunctionImplementation, TimeUnit)> = vec![
            (Arc::new(to_timestamp), Nanosecond),
            (Arc::new(to_timestamp_micros), Microsecond),
            (Arc::new(to_timestamp_millis), Millisecond),
            (Arc::new(to_timestamp_nanos), Nanosecond),
            (Arc::new(to_timestamp_seconds), Second),
        ];

        let mut nanos_builder = TimestampNanosecondArray::builder(2);
        let mut millis_builder = TimestampMillisecondArray::builder(2);
        let mut micros_builder = TimestampMicrosecondArray::builder(2);
        let mut sec_builder = TimestampSecondArray::builder(2);

        nanos_builder.append_value(1599572549190850000);
        millis_builder.append_value(1599572549190);
        micros_builder.append_value(1599572549190850);
        sec_builder.append_value(1599572549);

        let nanos_expected_timestamps = &nanos_builder.finish() as &dyn Array;
        let millis_expected_timestamps = &millis_builder.finish() as &dyn Array;
        let micros_expected_timestamps = &micros_builder.finish() as &dyn Array;
        let sec_expected_timestamps = &sec_builder.finish() as &dyn Array;

        for (func, time_unit) in funcs {
            // test UTF8
            let string_array = [
                ColumnarValue::Array(Arc::new(data.clone()) as ArrayRef),
                ColumnarValue::Scalar(ScalarValue::Utf8(Some("%s".to_string()))),
                ColumnarValue::Scalar(ScalarValue::Utf8(Some("%c".to_string()))),
                ColumnarValue::Scalar(ScalarValue::Utf8(Some("%+".to_string()))),
            ];
            let parsed_timestamps = func(&string_array)
                .expect("that to_timestamp with format args parsed values without error");
            if let ColumnarValue::Array(parsed_array) = parsed_timestamps {
                assert_eq!(parsed_array.len(), 1);
                match time_unit {
                    Nanosecond => {
                        assert_eq!(nanos_expected_timestamps, parsed_array.as_ref())
                    }
                    Millisecond => {
                        assert_eq!(millis_expected_timestamps, parsed_array.as_ref())
                    }
                    Microsecond => {
                        assert_eq!(micros_expected_timestamps, parsed_array.as_ref())
                    }
                    Second => {
                        assert_eq!(sec_expected_timestamps, parsed_array.as_ref())
                    }
                };
            } else {
                panic!("Expected a columnar array")
            }

            // test LargeUTF8
            let string_array = [
                ColumnarValue::Array(Arc::new(data.clone()) as ArrayRef),
                ColumnarValue::Scalar(ScalarValue::LargeUtf8(Some("%s".to_string()))),
                ColumnarValue::Scalar(ScalarValue::LargeUtf8(Some("%c".to_string()))),
                ColumnarValue::Scalar(ScalarValue::LargeUtf8(Some("%+".to_string()))),
            ];
            let parsed_timestamps = func(&string_array)
                .expect("that to_timestamp with format args parsed values without error");
            if let ColumnarValue::Array(parsed_array) = parsed_timestamps {
                assert_eq!(parsed_array.len(), 1);
                assert!(matches!(parsed_array.data_type(), Timestamp(_, None)));

                match time_unit {
                    Nanosecond => {
                        assert_eq!(nanos_expected_timestamps, parsed_array.as_ref())
                    }
                    Millisecond => {
                        assert_eq!(millis_expected_timestamps, parsed_array.as_ref())
                    }
                    Microsecond => {
                        assert_eq!(micros_expected_timestamps, parsed_array.as_ref())
                    }
                    Second => {
                        assert_eq!(sec_expected_timestamps, parsed_array.as_ref())
                    }
                };
            } else {
                panic!("Expected a columnar array")
            }

            // test other types
            let string_array = [
                ColumnarValue::Array(Arc::new(data.clone()) as ArrayRef),
                ColumnarValue::Scalar(ScalarValue::Int32(Some(1))),
                ColumnarValue::Scalar(ScalarValue::Int32(Some(2))),
                ColumnarValue::Scalar(ScalarValue::Int32(Some(3))),
            ];

            let expected = "Unsupported data type Int32 for function".to_string();
            let actual = func(&string_array).unwrap_err().to_string();
            assert_contains!(actual, expected);

            // test other types
            let string_array = [
                ColumnarValue::Array(Arc::new(data.clone()) as ArrayRef),
                ColumnarValue::Array(Arc::new(PrimitiveArray::<Int64Type>::new(
                    vec![1i64].into(),
                    None,
                )) as ArrayRef),
            ];

            let expected = "Unsupported data type".to_string();
            let actual = func(&string_array).unwrap_err().to_string();
            assert_contains!(actual, expected);
        }
    }
}
