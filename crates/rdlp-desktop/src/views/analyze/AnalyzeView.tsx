// AnalyzeView: media analysis workspace.
// States: empty (dropzone), loading (skeleton), loaded (formats table), search results, error.
// Playlist mode: shows EpisodeList instead of PresetToolbar + FormatsTable.

import { useMemo } from "react";
import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import { uiStore, setAnalyzeUrl } from "@/stores/uiStore";
import { searchStore } from "@/stores/searchStore";
import { formatsQueryOptions } from "@/api/formats";
import { ArrowLeft } from "lucide-react";
import { EmptyState } from "./EmptyState";
import { MediaHero } from "./MediaHero";
import { FormatsTable } from "./FormatsTable";
import { PresetToolbar } from "./PresetToolbar";
import { EpisodeList } from "./EpisodeList";
import { SearchResults } from "./SearchResults";
import { Skeleton } from "@/components/ui/skeleton";
import { AlertCircle } from "lucide-react";

export function AnalyzeView() {
    const analyzeUrl = useStore(uiStore, (s) => s.analyzeUrl);
    const searchQuery = useStore(searchStore, (s) => s.query);
    const searchSite = useStore(searchStore, (s) => s.site);
    const playlistBackUrl = useStore(uiStore, (s) => s.playlistBackUrl);

    const isSearchMode = useMemo(
        () => searchQuery.trim().length > 0 && !analyzeUrl,
        [searchQuery, analyzeUrl],
    );
    const cameFromSearch = useMemo(
        () => !!analyzeUrl && searchQuery.trim().length > 0 && !playlistBackUrl,
        [analyzeUrl, searchQuery, playlistBackUrl],
    );

    const {
        data: formats,
        isLoading,
        isError,
        error,
        refetch,
    } = useQuery(formatsQueryOptions(analyzeUrl));

    if (!analyzeUrl && !isSearchMode) {
        return <EmptyState />;
    }

    if (isSearchMode) {
        return <SearchResults query={searchQuery} site={searchSite} />;
    }

    if (isLoading) {
        return (
            <div className="p-4 flex flex-col gap-3">
                <Skeleton className="h-20 w-full rounded-[6px]" />
                <Skeleton className="h-8 w-full rounded-[4px]" />
                <Skeleton className="h-48 w-full rounded-[4px]" />
            </div>
        );
    }

    if (isError) {
        return (
            <div className="flex flex-col items-center justify-center h-full gap-4 p-8">
                <AlertCircle className="w-8 h-8 text-[#e85858]" />
                <div className="text-center">
                    <p className="text-[14px] font-medium text-[#eeeeee] mb-1">Failed to extract formats</p>
                    <p className="text-[12px] text-[#666666]">{(error as Error).message}</p>
                </div>
                <button
                    onClick={() => void refetch()}
                    className="px-4 py-1.5 rounded-[6px] bg-[#4a9eff] text-white text-[13px] font-medium hover:bg-[#3a8ef0] transition-colors"
                >
                    Retry
                </button>
            </div>
        );
    }

    if (!formats) {
        return <EmptyState />;
    }

    const isPlaylist = formats.is_playlist && formats.playlist !== null;

    return (
        <div className="flex flex-col h-full overflow-hidden">
            {/* Back to search results (only when not in episode drill-down) */}
            {cameFromSearch && (
                <button
                    onClick={() => setAnalyzeUrl(null)}
                    className="flex items-center gap-1.5 px-3 py-1.5 text-[12px] text-[#4a9eff] hover:text-[#3a8ef0] transition-colors border-b border-[#1a1a2e] shrink-0 bg-transparent cursor-pointer"
                >
                    <ArrowLeft className="w-3 h-3" />
                    Back to search results
                </button>
            )}

            {/* Back to episode list (when viewing a single episode from a playlist) */}
            {playlistBackUrl && (
                <button
                    onClick={() => {
                        uiStore.setState((prev) => ({
                            ...prev,
                            playlistBackUrl: null,
                        }));
                        setAnalyzeUrl(playlistBackUrl);
                    }}
                    className="flex items-center gap-1.5 px-3 py-1.5 text-[12px] text-[#4a9eff] hover:text-[#3a8ef0] transition-colors border-b border-[#1a1a2e] shrink-0 bg-transparent cursor-pointer"
                >
                    <ArrowLeft className="w-3 h-3" />
                    Back to episode list
                </button>
            )}

            {/* Media hero card */}
            <MediaHero data={formats} url={analyzeUrl ?? ""} />

            {/* Playlist mode: episode list */}
            {isPlaylist ? (
                <EpisodeList
                    episodes={formats.playlist!.entries}
                    playlistUrl={analyzeUrl ?? ""}
                />
            ) : (
                <>
                    {/* Single video mode: preset toolbar + formats table */}
                    <PresetToolbar formats={formats.formats} />
                    <div className="flex-1 overflow-hidden">
                        <FormatsTable formats={formats.formats} />
                    </div>
                </>
            )}
        </div>
    );
}
