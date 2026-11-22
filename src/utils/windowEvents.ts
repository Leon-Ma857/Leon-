export async function listenToWindow<T>(name: string, handler: (event: { payload: T }) => void) {
  const listener = (event: Event) => handler({ payload: (event as CustomEvent<T>).detail });
  window.addEventListener(name, listener);
  return () => window.removeEventListener(name, listener);
}
