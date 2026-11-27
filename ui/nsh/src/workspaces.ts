import { batch, signal, type Signal } from "nsh";
import { createNavStack, type NavStackState } from "nsh/base";

export type WorkspaceStack<R> = {
  readonly key: string;
  readonly nav: NavStackState<R>;
};

/**
 * Retain one independent `<NavStack>` per visited workspace without reordering
 * native stack children. Switching workspaces toggles visibility on the
 * workspace wrapper in place, preserving scroll, draft and editor state without
 * native detach/re-insert or fade transitions.
 */
export class Workspaces<R> {
  private readonly stacks = new Map<string, WorkspaceStack<R>>();
  readonly entries: Signal<readonly WorkspaceStack<R>[]>;
  readonly active: Signal<string>;

  constructor(key: string, root: R) {
    const initial: WorkspaceStack<R> = {
      key,
      nav: createNavStack<R>([root]),
    };
    this.stacks.set(key, initial);
    this.entries = signal<readonly WorkspaceStack<R>[]>([initial]);
    this.active = signal(key);
  }

  get root(): string {
    return this.active();
  }

  currentStack(): WorkspaceStack<R> {
    return this.stacks.get(this.active())!;
  }

  depth(): number {
    return this.currentStack().nav.routes().length;
  }

  route(): R {
    const routes = this.currentStack().nav.routes();
    return routes[routes.length - 1];
  }

  switch(key: string, make: () => R): void {
    const existing = this.stacks.get(key);
    if (existing) {
      if (this.active() !== key) this.active.set(key);
      return;
    }
    const created: WorkspaceStack<R> = {
      key,
      nav: createNavStack<R>([make()]),
    };
    this.stacks.set(key, created);
    batch(() => {
      this.entries.update((list) => [...list, created]);
      this.active.set(key);
    });
  }

  push(route: R): void {
    this.currentStack().nav.push(route);
  }

  replace(route: R): void {
    this.currentStack().nav.replace(route);
  }

  back(): void {
    const nav = this.currentStack().nav;
    if (nav.routes().length > 1) nav.pop();
  }
}
