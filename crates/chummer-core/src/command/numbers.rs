//! The numbers in a command must be usable game values (LB-44, LB-45):
//! integers within ±[`NUM_LIMIT`] (the range file integers are clamped
//! to) and floats finite and within ±[`FLOAT_LIMIT`].
//!
//! The check walks the command with a serde [`Serializer`] that only looks
//! at the numbers, so every field of every variant (and of the structs
//! they carry) is covered without listing them.

use serde::ser::{self, Serialize};

use crate::xml::NUM_LIMIT;

/// The largest magnitude of a decimal in a command: far above any amount
/// of nuyen, small enough that sums stay exact to the cent.
pub const FLOAT_LIMIT: f64 = 1e12;

/// What is wrong with the first bad number, for [`super::Rejected`].
#[derive(Debug)]
pub struct BadNumber(pub String);

impl std::fmt::Display for BadNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BadNumber {}

impl ser::Error for BadNumber {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        BadNumber(msg.to_string())
    }
}

/// `Err` with a reason when a number in `value` is out of range or not
/// finite.
pub fn check<T: Serialize + ?Sized>(value: &T) -> Result<(), BadNumber> {
    value.serialize(Checker)
}

fn int(v: i128) -> Result<(), BadNumber> {
    if v.unsigned_abs() > NUM_LIMIT.unsigned_abs() as u128 {
        return Err(BadNumber(format!("The number {v} is out of range (at most {NUM_LIMIT} either way).")));
    }
    Ok(())
}

fn float(v: f64) -> Result<(), BadNumber> {
    if !v.is_finite() {
        return Err(BadNumber(format!("{v} is not a number that can be used.")));
    }
    if v.abs() > FLOAT_LIMIT {
        return Err(BadNumber(format!("The number {v} is out of range (at most {FLOAT_LIMIT:e} either way).")));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Checker;

type R = Result<(), BadNumber>;

impl ser::Serializer for Checker {
    type Ok = ();
    type Error = BadNumber;
    type SerializeSeq = Checker;
    type SerializeTuple = Checker;
    type SerializeTupleStruct = Checker;
    type SerializeTupleVariant = Checker;
    type SerializeMap = Checker;
    type SerializeStruct = Checker;
    type SerializeStructVariant = Checker;

    fn serialize_bool(self, _: bool) -> R {
        Ok(())
    }
    fn serialize_i8(self, v: i8) -> R {
        int(v.into())
    }
    fn serialize_i16(self, v: i16) -> R {
        int(v.into())
    }
    fn serialize_i32(self, v: i32) -> R {
        int(v.into())
    }
    fn serialize_i64(self, v: i64) -> R {
        int(v.into())
    }
    // Unsigned numbers in commands are indices, slots and bytes (a
    // revert's snapshot): no rule adds them up.
    fn serialize_u8(self, _: u8) -> R {
        Ok(())
    }
    fn serialize_u16(self, _: u16) -> R {
        Ok(())
    }
    fn serialize_u32(self, _: u32) -> R {
        Ok(())
    }
    fn serialize_u64(self, _: u64) -> R {
        Ok(())
    }
    fn serialize_f32(self, v: f32) -> R {
        float(v.into())
    }
    fn serialize_f64(self, v: f64) -> R {
        float(v)
    }
    fn serialize_char(self, _: char) -> R {
        Ok(())
    }
    fn serialize_str(self, _: &str) -> R {
        Ok(())
    }
    fn serialize_bytes(self, _: &[u8]) -> R {
        Ok(())
    }
    fn serialize_none(self) -> R {
        Ok(())
    }
    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> R {
        v.serialize(self)
    }
    fn serialize_unit(self) -> R {
        Ok(())
    }
    fn serialize_unit_struct(self, _: &'static str) -> R {
        Ok(())
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> R {
        Ok(())
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _: &'static str, v: &T) -> R {
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(self, _: &'static str, _: u32, _: &'static str, v: &T) -> R {
        v.serialize(self)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Checker, BadNumber> {
        Ok(self)
    }
    fn serialize_tuple(self, _: usize) -> Result<Checker, BadNumber> {
        Ok(self)
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Checker, BadNumber> {
        Ok(self)
    }
    fn serialize_tuple_variant(self, _: &'static str, _: u32, _: &'static str, _: usize) -> Result<Checker, BadNumber> {
        Ok(self)
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Checker, BadNumber> {
        Ok(self)
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Checker, BadNumber> {
        Ok(self)
    }
    fn serialize_struct_variant(self, _: &'static str, _: u32, _: &'static str, _: usize) -> Result<Checker, BadNumber> {
        Ok(self)
    }
}

impl ser::SerializeSeq for Checker {
    type Ok = ();
    type Error = BadNumber;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> R {
        v.serialize(*self)
    }
    fn end(self) -> R {
        Ok(())
    }
}

impl ser::SerializeTuple for Checker {
    type Ok = ();
    type Error = BadNumber;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> R {
        v.serialize(*self)
    }
    fn end(self) -> R {
        Ok(())
    }
}

impl ser::SerializeTupleStruct for Checker {
    type Ok = ();
    type Error = BadNumber;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> R {
        v.serialize(*self)
    }
    fn end(self) -> R {
        Ok(())
    }
}

impl ser::SerializeTupleVariant for Checker {
    type Ok = ();
    type Error = BadNumber;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> R {
        v.serialize(*self)
    }
    fn end(self) -> R {
        Ok(())
    }
}

impl ser::SerializeMap for Checker {
    type Ok = ();
    type Error = BadNumber;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, v: &T) -> R {
        v.serialize(*self)
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, v: &T) -> R {
        v.serialize(*self)
    }
    fn end(self) -> R {
        Ok(())
    }
}

impl ser::SerializeStruct for Checker {
    type Ok = ();
    type Error = BadNumber;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, _: &'static str, v: &T) -> R {
        v.serialize(*self)
    }
    fn end(self) -> R {
        Ok(())
    }
}

impl ser::SerializeStructVariant for Checker {
    type Ok = ();
    type Error = BadNumber;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, _: &'static str, v: &T) -> R {
        v.serialize(*self)
    }
    fn end(self) -> R {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;

    #[test]
    fn examples_pass_and_bad_numbers_fail() {
        for c in Command::examples() {
            check(&c).unwrap_or_else(|e| panic!("{c:?}: {e}"));
        }
        for bad in [
            Command::SetNuyen { value: f64::NAN },
            Command::SetNuyen { value: f64::INFINITY },
            Command::SetNuyen { value: -1e13 },
            Command::SetKarma { value: i32::MAX },
            Command::SetGroupKarma { group: "Firearms".into(), value: -1_000_001 },
            Command::AddCustomDrug { name: "x".into(), grade: "y".into(), components: vec![("a".into(), i32::MIN)] },
        ] {
            assert!(check(&bad).is_err(), "{bad:?}");
        }
        // Limits are inclusive.
        check(&Command::SetKarma { value: NUM_LIMIT }).unwrap();
        check(&Command::SetNuyen { value: -FLOAT_LIMIT }).unwrap();
    }
}
