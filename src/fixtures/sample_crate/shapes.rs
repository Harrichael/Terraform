pub struct Rect {
    pub width: f64,
    pub height: f64,
}

pub fn area(r: &Rect) -> f64 {
    r.width * r.height
}

pub fn perimeter(r: &Rect) -> f64 {
    2.0 * (r.width + r.height)
}
