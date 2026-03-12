import { render, screen, act, fireEvent, createTestQueryClient } from "@/test/test-utils";
import { CommandBar } from "./CommandBar";
import { searchParamsAtom, resetSearchParams } from "../stores/searchParamsStore";
import { queryKeys } from "../query/queryKeys";
import { createRef } from "react";
import type { SearchSiteInfo } from "../types";

const mockProviders: SearchSiteInfo[] = [
    { name: "redtube", display_name: "RedTube" },
    { name: "xhamster", display_name: "xHamster" },
];

/** Render CommandBar with query cache pre-populated so useQuery resolves
 *  synchronously during render — no async microtask chain, no act() warnings. */
function renderCommandBar(props?: {
    activeTab?: string;
    isFetching?: boolean;
    onSearch?: () => void;
}) {
    const inputRef = createRef<HTMLInputElement>();
    const queryClient = createTestQueryClient();
    queryClient.setQueryData(queryKeys.providers(), mockProviders);
    return render(
        <CommandBar
            inputRef={inputRef as React.RefObject<HTMLInputElement>}
            activeTab={props?.activeTab ?? "search"}
            isFetching={props?.isFetching ?? false}
            onSearch={props?.onSearch ?? vi.fn()}
        />,
        { queryClient },
    );
}

beforeEach(() => {
    resetSearchParams();
});

afterEach(() => {
    act(() => resetSearchParams());
});

describe("CommandBar", () => {
    it("renders correct initial state (single render)", () => {
        renderCommandBar();
        expect(screen.getByPlaceholderText("Search videos...")).toBeInTheDocument();
        expect(screen.getByRole("combobox", { name: /search site/i })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /^search$/i })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /^search$/i })).toBeDisabled();
    });

    it("typing a query enables submit, shows clear, and updates store", () => {
        renderCommandBar();
        const input = screen.getByPlaceholderText("Search videos...");
        fireEvent.change(input, { target: { value: "test" } });
        expect(screen.getByRole("button", { name: /^search$/i })).not.toBeDisabled();
        expect(screen.getByRole("button", { name: /clear search/i })).toBeInTheDocument();
        expect(searchParamsAtom.state.query).toBe("test");
    });

    it("calls onSearch when form is submitted with non-empty query", () => {
        const onSearch = vi.fn();
        renderCommandBar({ onSearch });
        const input = screen.getByPlaceholderText("Search videos...");
        fireEvent.change(input, { target: { value: "cats" } });
        fireEvent.click(screen.getByRole("button", { name: /^search$/i }));
        expect(onSearch).toHaveBeenCalledTimes(1);
    });

    it("clears the query when clear button is clicked", () => {
        renderCommandBar();
        const input = screen.getByPlaceholderText("Search videos...");
        fireEvent.change(input, { target: { value: "dogs" } });
        fireEvent.click(screen.getByRole("button", { name: /clear search/i }));
        expect(input).toHaveValue("");
    });

    it("disables input and submit button while fetching", () => {
        renderCommandBar({ isFetching: true });
        expect(screen.getByPlaceholderText("Search videos...")).toBeDisabled();
        expect(screen.getByRole("button", { name: /^search$/i })).toBeDisabled();
    });
});
