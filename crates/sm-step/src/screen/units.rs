//! Units declared by representation contexts.

use super::part21::Param;
use super::{real_of, Args, CResult, ConvertError, Graph};

/// The units of one representation context.
#[derive(Debug, Clone, PartialEq)]
pub struct Units {
    /// Millimetres per file length unit.
    pub length: f64,
    /// Radians per file angle unit.
    pub angle: f64,
    pub length_name: String,
    /// Distance below which two points are the same, in millimetres.
    pub tolerance: f64,
}

impl Default for Units {
    fn default() -> Self {
        Units {
            length: 1.0,
            angle: 1.0,
            length_name: "millimetre".into(),
            tolerance: DEFAULT_TOLERANCE,
        }
    }
}

/// Used when a file states no uncertainty. Coarse enough for the gaps real
/// exporters leave between edges and faces.
const DEFAULT_TOLERANCE: f64 = 1e-5;

/// Uncertainty values are clamped into this range (millimetres): some
/// writers state 0, others a whole millimetre.
const MIN_TOLERANCE: f64 = 1e-7;
const MAX_TOLERANCE: f64 = 0.1;

/// Reads `GLOBAL_UNIT_ASSIGNED_CONTEXT` and
/// `GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT` from a representation context.
pub fn context_units(g: &Graph, ctx: u64) -> CResult<Units> {
    let entity = g.get(ctx)?;
    let mut units = Units::default();
    let mut found_length = false;
    if let Some(rec) = entity.record("GLOBAL_UNIT_ASSIGNED_CONTEXT") {
        for u in Args::new(ctx, rec).refs(0)? {
            match unit(g, u, 0)? {
                Unit::Length(mm, name) => {
                    units.length = mm;
                    units.length_name = name;
                    found_length = true;
                }
                Unit::Angle(rad) => units.angle = rad,
                Unit::Other => {}
            }
        }
    }
    if !found_length {
        return super::err(ctx, "no length unit");
    }
    if let Some(rec) = entity.record("GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT") {
        for u in Args::new(ctx, rec).refs(0)? {
            let Ok(r) = g.record(u, "UNCERTAINTY_MEASURE_WITH_UNIT") else {
                continue;
            };
            let a = Args::new(u, r);
            let (Ok(value), Ok(unit_id)) = (a.real(0), a.reference(1)) else {
                continue;
            };
            if let Ok(Unit::Length(mm, _)) = unit(g, unit_id, 0) {
                let tol = value.abs() * mm;
                if tol.is_finite() {
                    units.tolerance = tol.clamp(MIN_TOLERANCE, MAX_TOLERANCE);
                }
            }
        }
    }
    Ok(units)
}

enum Unit {
    /// Millimetres per unit, and the unit's name.
    Length(f64, String),
    /// Radians per unit.
    Angle(f64),
    Other,
}

fn unit(g: &Graph, id: u64, depth: usize) -> CResult<Unit> {
    if depth > 8 {
        return super::err(id, "unit conversions nested too deeply");
    }
    let entity = g.get(id)?;
    let is_length = entity.has("LENGTH_UNIT");
    let is_angle = entity.has("PLANE_ANGLE_UNIT");
    if let Some(si) = entity.record("SI_UNIT") {
        let a = Args::new(id, si);
        // A simple SI_UNIT instance has the NAMED_UNIT dimensions first.
        let offset = usize::from(si.params.len() == 3);
        let prefix = match a.param(offset)? {
            Param::Enum(p) => Some(p.as_str()),
            _ => None,
        };
        let name = match a.param(offset + 1)? {
            Param::Enum(n) => n.as_str(),
            _ => return super::err(id, "SI_UNIT without a name"),
        };
        let factor = match prefix {
            None => 1.0,
            Some(p) => match si_prefix(p) {
                Some(f) => f,
                None => return super::err(id, format!("unknown SI prefix {p}")),
            },
        };
        return Ok(match name {
            "METRE" => Unit::Length(
                factor * 1000.0,
                format!(
                    "{}metre",
                    prefix.map(|p| p.to_ascii_lowercase()).unwrap_or_default()
                ),
            ),
            "RADIAN" => Unit::Angle(factor),
            _ => Unit::Other,
        });
    }
    if let Some(cb) = entity.record("CONVERSION_BASED_UNIT") {
        if !is_length && !is_angle {
            return Ok(Unit::Other);
        }
        let a = Args::new(id, cb);
        let name = a.string(0);
        let measure = a.reference(1)?;
        let m = g.get(measure)?;
        let rec = m
            .records()
            .iter()
            .find(|r| r.name.ends_with("MEASURE_WITH_UNIT") && r.params.len() >= 2)
            .ok_or_else(|| ConvertError {
                entity: measure,
                message: "expected a measure with unit".into(),
            })?;
        let ma = Args::new(measure, rec);
        let value = real_of(ma.param(0)?).ok_or_else(|| ConvertError {
            entity: measure,
            message: "measure is not a number".into(),
        })?;
        let base = unit(g, ma.reference(1)?, depth + 1)?;
        let check = |v: f64| {
            if v.is_finite() && v > 0.0 {
                Ok(v)
            } else {
                super::err(id, "unit conversion factor must be positive")
            }
        };
        return Ok(match base {
            Unit::Length(mm, _) => Unit::Length(check(value * mm)?, name.to_ascii_lowercase()),
            Unit::Angle(rad) => Unit::Angle(check(value * rad)?),
            Unit::Other => Unit::Other,
        });
    }
    Ok(Unit::Other)
}

fn si_prefix(p: &str) -> Option<f64> {
    Some(match p {
        "EXA" => 1e18,
        "PETA" => 1e15,
        "TERA" => 1e12,
        "GIGA" => 1e9,
        "MEGA" => 1e6,
        "KILO" => 1e3,
        "HECTO" => 1e2,
        "DECA" => 1e1,
        "DECI" => 1e-1,
        "CENTI" => 1e-2,
        "MILLI" => 1e-3,
        "MICRO" => 1e-6,
        "NANO" => 1e-9,
        "PICO" => 1e-12,
        "FEMTO" => 1e-15,
        "ATTO" => 1e-18,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::{part21, Limits};
    use super::*;

    fn units_of(data: &str) -> CResult<Units> {
        let text = format!("ISO-10303-21;HEADER;ENDSEC;DATA;{data}ENDSEC;END-ISO-10303-21;");
        let limits = Limits::default();
        let ex = part21::parse(text.as_bytes(), &limits).unwrap();
        context_units(&Graph { ex: &ex }, 1)
    }

    const CTX: &str =
        "#1=(GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#9)) \
        GLOBAL_UNIT_ASSIGNED_CONTEXT((#2,#3,#4)) REPRESENTATION_CONTEXT('',''));";

    #[test]
    fn reads_si_and_conversion_based_units() {
        let u = units_of(&format!(
            "{CTX}#2=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));\
             #3=(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.));\
             #4=(NAMED_UNIT(*)SI_UNIT($,.STERADIAN.)SOLID_ANGLE_UNIT());\
             #9=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-07),#2,'','');"
        ))
        .unwrap();
        assert_eq!(
            (u.length, u.angle, u.length_name.as_str()),
            (1.0, 1.0, "millimetre")
        );
        assert_eq!(u.tolerance, 1e-7);

        // Inches and degrees, as SolidWorks and Inventor write them.
        let u = units_of(&format!(
            "{CTX}#2=(CONVERSION_BASED_UNIT('INCH',#5)LENGTH_UNIT()NAMED_UNIT(#6));\
             #5=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#7);\
             #6=DIMENSIONAL_EXPONENTS(1.,0.,0.,0.,0.,0.,0.);\
             #7=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));\
             #3=(CONVERSION_BASED_UNIT('DEGREE',#8)NAMED_UNIT(#6)PLANE_ANGLE_UNIT());\
             #8=PLANE_ANGLE_MEASURE_WITH_UNIT(PLANE_ANGLE_MEASURE(0.0174532925),#10);\
             #10=(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.));\
             #4=(NAMED_UNIT(*)SI_UNIT($,.STERADIAN.)SOLID_ANGLE_UNIT());\
             #9=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(0.0001),#2,'','');"
        ))
        .unwrap();
        assert_eq!(u.length, 25.4);
        assert_eq!(u.length_name, "inch");
        assert!((u.angle - 0.0174532925).abs() < 1e-12);
        assert!((u.tolerance - 0.00254).abs() < 1e-12);

        // Metres, with no uncertainty.
        let u = units_of(
            "#1=(GLOBAL_UNIT_ASSIGNED_CONTEXT((#2)) REPRESENTATION_CONTEXT('',''));\
             #2=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT($,.METRE.));",
        )
        .unwrap();
        assert_eq!((u.length, u.tolerance), (1000.0, DEFAULT_TOLERANCE));
    }

    #[test]
    fn rejects_bad_units() {
        // No length unit.
        assert!(
            units_of("#1=(GLOBAL_UNIT_ASSIGNED_CONTEXT(()) REPRESENTATION_CONTEXT('',''));")
                .is_err()
        );
        // A conversion unit referring to itself.
        assert!(units_of(
            "#1=(GLOBAL_UNIT_ASSIGNED_CONTEXT((#2)) REPRESENTATION_CONTEXT('',''));\
             #2=(CONVERSION_BASED_UNIT('LOOP',#3)LENGTH_UNIT()NAMED_UNIT(*));\
             #3=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(2.),#2);"
        )
        .is_err());
        // Zero and negative factors.
        for v in ["0.", "-25.4"] {
            assert!(units_of(&format!(
                "#1=(GLOBAL_UNIT_ASSIGNED_CONTEXT((#2)) REPRESENTATION_CONTEXT('',''));\
                 #2=(CONVERSION_BASED_UNIT('X',#3)LENGTH_UNIT()NAMED_UNIT(*));\
                 #3=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE({v}),#4);\
                 #4=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));"
            ))
            .is_err());
        }
        // Unknown prefix.
        assert!(units_of(
            "#1=(GLOBAL_UNIT_ASSIGNED_CONTEXT((#2)) REPRESENTATION_CONTEXT('',''));\
             #2=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.BOGO.,.METRE.));"
        )
        .is_err());
    }
}
