/** 同一窗口内，供应商成功变更后通知所有清单消费者重新读取。 */
const listeners = new Set<() => void>();

export function onVendorsChanged(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

export function vendorsChanged(): void {
  for (const listener of listeners) listener();
}
