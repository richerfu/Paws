/** Retain route identities while switching workspaces, without a Back history
 * between them. NSH keeps those pages and their native scroll/editor state. */
export class Workspaces<R> {
  private readonly stacks = new Map<string, R[]>();
  private active: string;
  constructor(key: string, root: R) {
    this.active = key;
    this.stacks.set(key, [root]);
  }
  get depth() { return this.stacks.get(this.active)!.length; }
  get root() { return this.active; }
  switch(key: string, make: () => R) {
    if (!this.stacks.has(key)) this.stacks.set(key, [make()]);
    this.active = key;
    return this.routes();
  }
  push(route: R) {
    this.stacks.get(this.active)!.push(route);
    return this.routes();
  }
  replace(route: R) {
    const stack = this.stacks.get(this.active)!;
    stack[stack.length - 1] = route;
    return this.routes();
  }
  back() {
    if (this.depth > 1) this.stacks.get(this.active)!.pop();
    return this.routes();
  }
  routes() {
    return [...this.stacks.entries()].filter(([key]) => key !== this.active)
      .flatMap(([, routes]) => routes).concat(this.stacks.get(this.active)!);
  }
}
