//! Length units (turf-compatible names). Everything is converted to metres
//! internally; angular units (`radians`, `degrees`) are rejected because they
//! only make sense on a sphere.

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Units {
    Meters,
    Millimeters,
    Centimeters,
    Kilometers,
    Miles,
    NauticalMiles,
    Inches,
    Yards,
    Feet,
}

impl Units {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "" | "kilometers" | "kilometres" | "km" => Units::Kilometers,
            "meters" | "metres" | "m" => Units::Meters,
            "millimeters" | "millimetres" | "mm" => Units::Millimeters,
            "centimeters" | "centimetres" | "cm" => Units::Centimeters,
            "miles" | "mi" => Units::Miles,
            "nauticalmiles" | "nmi" => Units::NauticalMiles,
            "inches" | "in" => Units::Inches,
            "yards" | "yd" => Units::Yards,
            "feet" | "ft" => Units::Feet,
            other => return Err(Error::UnsupportedUnit(other.to_string())),
        })
    }

    /// Metres per one unit.
    pub fn meters_per_unit(self) -> f64 {
        match self {
            Units::Meters => 1.0,
            Units::Millimeters => 0.001,
            Units::Centimeters => 0.01,
            Units::Kilometers => 1000.0,
            Units::Miles => 1609.344,
            Units::NauticalMiles => 1852.0,
            Units::Inches => 0.0254,
            Units::Yards => 0.9144,
            Units::Feet => 0.3048,
        }
    }

    pub fn to_meters(self, v: f64) -> f64 {
        v * self.meters_per_unit()
    }

    pub fn from_meters(self, m: f64) -> f64 {
        m / self.meters_per_unit()
    }
}
