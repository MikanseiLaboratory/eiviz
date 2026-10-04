/// Progress curve. `Hold` stays at the start value until the segment ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Curve {
    Linear,
    In,
    Out,
    InOut,
    Smoothstep,
    Bezier { x1: f32, y1: f32, x2: f32, y2: f32 },
    Hold,
}

impl Curve {
    pub const BEZIER: u32 = crate::abi::EASING_BEZIER;
    pub const HOLD: u32 = crate::abi::EASING_HOLD;

    /// Rejects an unknown kind and Bezier handles whose X is outside `[0, 1]`.
    pub fn try_new(kind: u32, x1: f32, y1: f32, x2: f32, y2: f32) -> Result<Self, &'static str> {
        match kind {
            0 => Ok(Self::Linear),
            1 => Ok(Self::In),
            2 => Ok(Self::Out),
            3 => Ok(Self::InOut),
            4 => Ok(Self::Smoothstep),
            Self::BEZIER => {
                if !valid_bezier(x1, y1, x2, y2) {
                    return Err("bezier handles are outside the supported range");
                }
                Ok(Self::Bezier { x1, y1, x2, y2 })
            }
            Self::HOLD => Ok(Self::Hold),
            _ => Err("unknown easing"),
        }
    }

    pub fn eval(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::In => t * t * t,
            Self::Out => 1.0 - (1.0 - t).powi(3),
            Self::InOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Self::Smoothstep => t * t * (3.0 - 2.0 * t),
            Self::Bezier { x1, y1, x2, y2 } => bezier_y(x1, y1, x2, y2, t),
            Self::Hold => {
                if t >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

fn valid_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> bool {
    [x1, y1, x2, y2].iter().all(|v| v.is_finite())
        && (0.0..=1.0).contains(&x1)
        && (0.0..=1.0).contains(&x2)
}

/// CSS `cubic-bezier`: solve X(t) = x, then return Y(t).
fn bezier_y(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let cx = 3.0 * x1;
    let bx = 3.0 * (x2 - x1) - cx;
    let ax = 1.0 - cx - bx;
    let cy = 3.0 * y1;
    let by = 3.0 * (y2 - y1) - cy;
    let ay = 1.0 - cy - by;
    let t = solve_bezier_x(ax, bx, cx, x);
    sample(ay, by, cy, t)
}

fn sample(a: f32, b: f32, c: f32, t: f32) -> f32 {
    ((a * t + b) * t + c) * t
}

fn sample_derivative(a: f32, b: f32, c: f32, t: f32) -> f32 {
    (3.0 * a * t + 2.0 * b) * t + c
}

fn solve_bezier_x(ax: f32, bx: f32, cx: f32, x: f32) -> f32 {
    let mut t = x;
    for _ in 0..8 {
        let err = sample(ax, bx, cx, t) - x;
        if err.abs() < 1e-6 {
            return t.clamp(0.0, 1.0);
        }
        let slope = sample_derivative(ax, bx, cx, t);
        if slope.abs() < 1e-6 {
            break;
        }
        t -= err / slope;
    }
    let mut lo = 0.0f32;
    let mut hi = 1.0f32;
    t = x.clamp(0.0, 1.0);
    for _ in 0..24 {
        let est = sample(ax, bx, cx, t);
        if (est - x).abs() < 1e-6 {
            return t;
        }
        if x > est {
            lo = t;
        } else {
            hi = t;
        }
        t = (hi + lo) * 0.5;
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_and_named_curves_match_the_previous_polynomials() {
        for kind in 0..5 {
            let curve = Curve::try_new(kind, 0.0, 0.0, 1.0, 1.0).unwrap();
            assert!((curve.eval(0.0) - 0.0).abs() < 1e-6);
            assert!((curve.eval(1.0) - 1.0).abs() < 1e-6);
        }
        let ease_in = Curve::In;
        assert!((ease_in.eval(0.5) - 0.125).abs() < 1e-6);
        let smooth = Curve::Smoothstep;
        assert!((smooth.eval(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn bezier_linear_and_symmetric_ease_are_monotonic() {
        let linear = Curve::try_new(Curve::BEZIER, 0.0, 0.0, 1.0, 1.0).unwrap();
        let ease = Curve::try_new(Curve::BEZIER, 0.42, 0.0, 0.58, 1.0).unwrap();
        let mut prev_l = -0.01f32;
        let mut prev_e = -0.01f32;
        for i in 0..=20 {
            let x = i as f32 / 20.0;
            let y_l = linear.eval(x);
            let y_e = ease.eval(x);
            assert!((y_l - x).abs() < 1e-3, "linear bezier at {x} was {y_l}");
            assert!(y_e + 1e-4 >= prev_e, "ease-in-out decreased at {x}");
            assert!(y_l + 1e-4 >= prev_l);
            prev_l = y_l;
            prev_e = y_e;
        }
        assert!((ease.eval(0.5) - 0.5).abs() < 1e-3);
        assert!((ease.eval(0.0)).abs() < 1e-6);
        assert!((ease.eval(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hold_stays_until_the_end() {
        assert_eq!(Curve::Hold.eval(0.0), 0.0);
        assert_eq!(Curve::Hold.eval(0.999), 0.0);
        assert_eq!(Curve::Hold.eval(1.0), 1.0);
    }

    #[test]
    fn rejects_bezier_x_outside_unit_interval_and_unknown_kinds() {
        assert!(Curve::try_new(Curve::BEZIER, -0.1, 0.0, 1.0, 1.0).is_err());
        assert!(Curve::try_new(Curve::BEZIER, 0.0, 0.0, 1.2, 1.0).is_err());
        assert!(Curve::try_new(9, 0.0, 0.0, 1.0, 1.0).is_err());
        assert!(Curve::try_new(Curve::BEZIER, 0.2, -0.4, 0.8, 1.4).is_ok());
    }
}
