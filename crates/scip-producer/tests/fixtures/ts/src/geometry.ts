export class Point {
  constructor(public x: number, public y: number) {}

  magnitude(): number {
    return Math.sqrt(this.x * this.x + this.y * this.y);
  }

  describe(): string {
    return `(${this.x}, ${this.y})`;
  }
}

export function origin(): Point {
  return new Point(0, 0);
}

export const banner = "🚀"; export function shout(): string { return banner; }
