package main

import (
	"fmt"

	"example.com/fixture/geometry"
)

func describe(p geometry.Point) string {
	return fmt.Sprintf("magnitude %v", p.Magnitude())
}

func main() {
	fmt.Println(describe(geometry.New(3, 4)))
}
