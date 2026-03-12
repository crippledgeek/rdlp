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
    it("renders the search input", () => {
        renderCommandBar();
        expect(screen.getByPlaceholderText("Search videos...")).toBeInTheDocument();
    });

    it("renders the site selector trigger", () => {
        renderCommandBar();
        expect(screen.getByRole("combobox", { name: /search site/i })).toBeInTheDocument();
    });

    it("renders the submit button", () => {
        renderCommandBar();
        expect(screen.getByRole("button", { name: /^search$/i })).toBeInTheDocument();
    });

    it("submit button is disabled when query is empty and no category filter", () => {
        renderCommandBar();
        const btn = screen.getByRole("button", { name: /^search$/i });
        expect(btn).toBeDisabled();
    });

    it("submit button is enabled after typing a query", () => {
        renderCommandBar();
        const input = screen.getByPlaceholderText("Search videos...");
        fireEvent.change(input, { target: { value: "test" } });
        expect(screen.getByRole("button", { name: /^search$/i })).not.toBeDisabled();
    });

    it("calls onSearch when form is submitted with non-empty query", () => {
        const onSearch = vi.fn();
        renderCommandBar({ onSearch });
        const input = screen.getByPlaceholderText("Search videos...");
        fireEvent.change(input, { target: { value: "cats" } });
        fireEvent.click(screen.getByRole("button", { name: /^search$/i }));
        expect(onSearch).toHaveBeenCalledTimes(1);
    });

    it("shows clear button when query has text", () => {
        renderCommandBar();
        const input = screen.getByPlaceholderText("Search videos...");
        fireEvent.change(input, { target: { value: "dogs" } });
        expect(screen.getByRole("button", { name: /clear search/i })).toBeInTheDocument();
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

    it("updates store query on input change", () => {
        renderCommandBar();
        const input = screen.getByPlaceholderText("Search videos...");
        fireEvent.change(input, { target: { value: "hello" } });
        expect(searchParamsAtom.state.query).toBe("hello");
    });
});
