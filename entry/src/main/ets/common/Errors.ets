/**
 * Shared error description helper for the ArkTS side. Tolerant of the many
 * error shapes thrown by HarmonyOS platform APIs.
 */
interface PawsErrorLike {
  code?: number;
  message?: string;
  name?: string;
  stack?: string;
}

/** User-facing bridge errors retain their cause, without diagnostic stacks. */
export function describeUserError(err: Error | Object | string | number | boolean | null | undefined): string {
  if (err instanceof Error) {
    const like = err as PawsErrorLike;
    const code = typeof like.code === 'number' ? `code=${like.code} ` : '';
    return `${code}${err.name}: ${err.message}`;
  }
  if (err === null || err === undefined || typeof err !== 'object') {
    return describeError(err);
  }
  const like = err as PawsErrorLike;
  if (typeof like.message === 'string') {
    const code = typeof like.code === 'number' ? `code=${like.code} ` : '';
    const name = typeof like.name === 'string' && like.name.length > 0 ? `${like.name}: ` : '';
    return `${code}${name}${like.message}`;
  }
  // Preserve unknown structured causes too. A diagnostic stack is not the
  // cause; callers which need it still use describeError for their logs.
  const json = JSON.stringify(err, (key: string, value: Object): Object | undefined =>
    key === 'stack' ? undefined : value);
  return json && json !== '{}' ? json : err.toString();
}

export function describeError(err: Error | Object | string | number | boolean | null | undefined): string {
  if (err instanceof Error) {
    return `${err.name}: ${err.message}${err.stack ? ` ${err.stack}` : ''}`;
  }
  if (typeof err === 'string') {
    return err;
  }
  if (typeof err === 'number' || typeof err === 'boolean') {
    return err.toString();
  }
  if (!err) {
    return '';
  }
  const like = err as PawsErrorLike;
  if (typeof like.message === 'string' && like.message.length > 0) {
    const code = typeof like.code === 'number' ? `code=${like.code} ` : '';
    const name = typeof like.name === 'string' && like.name.length > 0 ? `${like.name}: ` : '';
    return `${code}${name}${like.message}${like.stack ? ` ${like.stack}` : ''}`;
  }
  const json = JSON.stringify(err);
  return json && json !== '{}' ? json : err.toString();
}
