import { render, screen, act, createTestQueryClient } from "@/test/test-utils";
import { FilterBar } from "./FilterBar";
import { resetSearchParams, setSearchParam, searchParamsAtom } from "../stores/searchParamsStore";
import { queryKeys } from "../query/queryKeys";
import type { SearchFilterDescriptor } from "../types";

const mockFilters: SearchFilterDescriptor[] = [
    {
        key: "ordering",
        display_name: "Sort",
        default: "relevance",
        allowed_values: [
            { value: "relevance", label: "Relevance" },
            { value: "newest", label: "Newest" },
            { value: "mostviewed", label: "Most Viewed" },
        ],
    },
    {
        key: "period",
        display_name: "Period",
        default: "alltime",
        allowed_values: [
            { value: "alltime", label: "All Time" },
            { value: "weekly", label: "Weekly" },
        ],
    },
];

/** Render FilterBar with query cache pre-populated so useQuery resolves
 *  synchronously during render — no async microtask chain, no act() warnings. */
function renderFilterBar(
    props?: {
        isFetching?: boolean;
        hasSearchData?: boolean;
        onSearch?: () => void;
    },
    filters: SearchFilterDescriptor[] = mockFilters,
) {
    const queryClient = createTestQueryClient();
    queryClient.setQueryData(queryKeys.filters("redtube"), filters);
    return render(
        <FilterBar
            isFetching={props?.isFetching ?? false}
            hasSearchData={props?.hasSearchData ?? false}
            onSearch={props?.onSearch ?? vi.fn()}
        />,
        { queryClient },
    );
}

beforeEach(() => {
    resetSearchParams();
    setSearchParam("site", "redtube");
});

afterEach(() => {
    act(() => resetSearchParams());
});

describe("FilterBar", () => {
    it("renders null when no filter descriptors are available", () => {
        const { container } = renderFilterBar({}, []);
        expect(container.firstChild).toBeNull();
    });

    it("renders filter selects once descriptors load", () => {
        renderFilterBar();
        expect(screen.getByRole("combobox", { name: /sort/i })).toBeInTheDocument();
        expect(screen.getByRole("combobox", { name: /period/i })).toBeInTheDocument();
    });

    it("shows reset button when a filter differs from its default", () => {
        renderFilterBar({ hasSearchData: true });

        // Manually set a non-default filter on the atom to simulate a change.
        // Wrap in act() because the atom mutation triggers useStore re-render.
        act(() => {
            searchParamsAtom.setState((prev) => ({
                ...prev,
                filters: [{ key: "ordering", value: "newest" }],
                hasUserFilters: true,
            }));
        });

        expect(screen.getByRole("button", { name: /reset all filters/i })).toBeInTheDocument();
    });

    it("does not show reset button when all filters are at defaults", () => {
        renderFilterBar();
        expect(screen.queryByRole("button", { name: /reset all filters/i })).not.toBeInTheDocument();
    });

    it("disables filter selects while fetching", () => {
        renderFilterBar({ isFetching: true });
        const selects = screen.getAllByRole("combobox");
        for (const s of selects) {
            expect(s).toBeDisabled();
        }
    });
});
