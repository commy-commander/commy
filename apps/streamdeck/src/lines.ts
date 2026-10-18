export const MAX_LINE_CHARS = 1 << 20;

export class LineBuffer {
  #pending = "";

  push(chunk: string): string[] {
    this.#pending += chunk;
    const lines = this.#pending.split("\n");
    this.#pending = lines.pop() ?? "";
    if (this.#pending.length > MAX_LINE_CHARS) {
      this.#pending = "";
      throw new Error("line too long");
    }
    return lines.map(line => line.replace(/\r$/, "")).filter(line => line.length > 0);
  }
}
