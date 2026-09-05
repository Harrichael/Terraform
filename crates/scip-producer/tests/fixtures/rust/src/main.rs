mod geometry;
mod util;

use geometry::Point;

fn main() {
    let p = Point::new(3.0, 4.0);
    println!("{}", describe(&p));
    util::greet();
}

fn describe(p: &Point) -> String {
    format!("{} has magnitude {}", p, p.magnitude())
}
