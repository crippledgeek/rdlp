// Tests for withDevtools.
//
// This HOC is NOT dev-only code. The Vite plugin strips the devtools imports
// and JSX out of it for a production build, but the wrapper itself survives and
// ships — so its prop forwarding and its identity are production behaviour and
// are tested as such.
//
// The devtools modules are mocked rather than rendered: the real shell mounts a
// floating trigger, reads localStorage and opens an event bus, none of which
// this file's behaviour depends on.

import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import type { ComponentType } from "react";

vi.mock("@tanstack/react-devtools", () => ({
    TanStackDevtools: () => <div data-testid="devtools-shell" />,
}));

vi.mock("@tanstack/react-query-devtools", () => ({
    ReactQueryDevtoolsPanel: () => <div data-testid="query-panel" />,
}));

const { withDevtools } = await import("./withDevtools");

describe("withDevtools", () => {
    it("renders the wrapped component", () => {
        const Wrapped = () => <div data-testid="app">app</div>;
        const Root = withDevtools(Wrapped);

        render(<Root />);

        expect(screen.getByTestId("app")).toBeInTheDocument();
    });

    it("forwards props through to the wrapped component", () => {
        // The generic signature is the whole reason this HOC can wrap `App`
        // without widening its prop type; a wrapper that dropped props would
        // still render, so assert the value arrives.
        const Wrapped = ({ label }: { label: string }) => <div>{label}</div>;
        const Root = withDevtools(Wrapped);

        render(<Root label="forwarded" />);

        expect(screen.getByText("forwarded")).toBeInTheDocument();
    });

    it("mounts exactly one devtools shell", () => {
        const Root = withDevtools(() => <div />);

        render(<Root />);

        expect(screen.getAllByTestId("devtools-shell")).toHaveLength(1);
    });

    // displayName has three branches and they are the component's identity in
    // the React devtools tree, where an unnamed wrapper between the root and
    // the app is exactly the confusion this avoids.
    it("derives displayName from an explicit displayName", () => {
        const Wrapped: ComponentType = () => <div />;
        Wrapped.displayName = "ExplicitName";

        expect(withDevtools(Wrapped).displayName).toBe("withDevtools(ExplicitName)");
    });

    it("falls back to the function name when there is no displayName", () => {
        function NamedByFunction() {
            return <div />;
        }

        expect(withDevtools(NamedByFunction).displayName).toBe(
            "withDevtools(NamedByFunction)",
        );
    });

    it("falls back to Component when there is neither", () => {
        const Anonymous: ComponentType = () => <div />;
        // An arrow assigned to a const infers `name` from the binding, so the
        // no-name branch is unreachable without clearing it explicitly. It is
        // reachable in real code — an inline or re-exported component can
        // arrive with an empty name.
        Object.defineProperty(Anonymous, "name", { value: "" });

        expect(withDevtools(Anonymous).displayName).toBe("withDevtools(Component)");
    });
});
