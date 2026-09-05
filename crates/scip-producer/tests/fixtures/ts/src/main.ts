import { Point, origin } from "./geometry";

function report(p: Point): string {
  return `${p.describe()} has magnitude ${p.magnitude()}`;
}

export function main(): void {
  const p = new Point(3, 4);
  console.log(report(p));
  console.log(report(origin()));
}
