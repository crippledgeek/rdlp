// Typed Tauri invoke mock with fixture routing.
//
// Use `setInvokeHandler` in tests to register command handlers that
// return fixture data. Unregistered commands resolve to undefined by default.

type InvokeHandler = (args?: Record<string, unknown>) => unknown;

const handlers: Map<string, InvokeHandler> = new Map();

/** Register a handler for a specific Tauri command name. */
export function setInvokeHandler(command: string, handler: InvokeHandler): void {
    handlers.set(command, handler);
}

/** Remove all registered handlers (call in afterEach). */
export function clearInvokeHandlers(): void {
    handlers.clear();
}

/** The mock invoke function installed as `@tauri-apps/api/core` invoke. */
export async function mockInvoke<T = unknown>(
    command: string,
    args?: Record<string, unknown>,
): Promise<T> {
    const handler = handlers.get(command);
    if (handler) {
        return handler(args) as T;
    }
    return undefined as T;
}

type ListenHandler = (payload: unknown) => void;
type UnlistenFn = () => void;

const eventListeners: Map<string, ListenHandler[]> = new Map();

/** The mock listen function installed as `@tauri-apps/api/event` listen. */
export async function mockListen<T = unknown>(
    event: string,
    handler: (event: { payload: T }) => void,
): Promise<UnlistenFn> {
    const wrapped = (payload: unknown) => handler({ payload: payload as T });
    const listeners = eventListeners.get(event) ?? [];
    listeners.push(wrapped);
    eventListeners.set(event, listeners);

    return () => {
        const current = eventListeners.get(event) ?? [];
        const idx = current.indexOf(wrapped);
        if (idx !== -1) current.splice(idx, 1);
    };
}

/** The mock emit function installed as `@tauri-apps/api/event` emit. */
export async function mockEmit(event: string, payload?: unknown): Promise<void> {
    const listeners = eventListeners.get(event) ?? [];
    for (const handler of listeners) {
        handler(payload);
    }
}

/** Remove all registered event listeners (call in afterEach). */
export function clearEventListeners(): void {
    eventListeners.clear();
}
