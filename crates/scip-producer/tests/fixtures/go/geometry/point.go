package geometry

import "math"

type Point struct {
	X, Y float64
}

func New(x, y float64) Point {
	return Point{X: x, Y: y}
}

func (p Point) Magnitude() float64 {
	return math.Sqrt(p.X*p.X + p.Y*p.Y)
}
