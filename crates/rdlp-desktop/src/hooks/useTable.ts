// Thin wrapper isolating useReactTable from React Compiler.
//
// useReactTable triggers an "IncompatibleLibrary" bail-out in the React
// Compiler, preventing it from optimising the calling function.  By
// confining the call to this tiny hook the bail-out stays here, while
// every consumer (useSearchPage, FormatDialog, …) gets full compiler
// memoisation.

import { useReactTable } from "@tanstack/react-table";
import type { TableOptions } from "@tanstack/react-table";

export function useTable<T>(options: TableOptions<T>) {
    return useReactTable(options);
}
