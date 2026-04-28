// E2E tests for the search flow.
//
// The CommandBar at the top of the app accepts both URLs (analyze) and free-text
// queries (search). The site selector is hidden via CSS until the input is in
// search mode (i.e., the displayValue is non-empty and does not look like a URL).

import { setupTauriMock } from "../support/e2e";

const INPUT = 'input[aria-label="URL or search query"]';
const CLEAR_BUTTON = 'button[aria-label="Clear input"]';

describe("Search flow", () => {
    it("shows the unified URL/search input on app load", () => {
        cy.get(INPUT).should("be.visible");
    });

    it("shows the site selector with provider options once in search mode", () => {
        // Type a non-URL query and verify React commits it to the input value
        // — the site-selector wrapper hides via `display: none` until then.
        cy.get(INPUT).type("test").should("have.value", "test");
        // Open the site selector by clicking its trigger. The wrapper has
        // role="presentation"; its trigger is the only listbox-popup button
        // visible on the page after entering search mode.
        cy.get('[aria-haspopup="listbox"]').filter(":visible").last().click();
        // React Aria portals the listbox; assert existence rather than
        // visibility to avoid timing flakes with the popover positioning.
        cy.contains("RedTube").should("exist");
    });

    it("shows search results after submitting a query", () => {
        cy.get(INPUT).type("test query{enter}");

        cy.contains("Test Video One").should("be.visible");
        cy.contains("Test Video Two").should("be.visible");
    });

    it("shows 'Load more' button when there are more pages", () => {
        cy.get(INPUT).type("test query{enter}");

        cy.contains("button", "Load more").should("be.visible");
    });

    it("loads the next page when 'Load more' is clicked", () => {
        cy.get(INPUT).type("test query{enter}");

        cy.contains("Test Video One").should("be.visible");
        cy.contains("button", "Load more").click();

        cy.contains("Test Video Three").should("be.visible");
        cy.contains("All results loaded").should("be.visible");
    });

    it("shows the result count after a search", () => {
        cy.get(INPUT).type("test query{enter}");

        // SearchResults renders "{count} of ~{total} results" once data loads.
        cy.contains(/\d+ of ~\d+ results/).should("be.visible");
    });

    it("clears the input when the clear button is clicked", () => {
        cy.get(INPUT).type("test query");

        cy.get(CLEAR_BUTTON).should("be.visible").click();

        cy.get(INPUT).should("have.value", "");
    });

    it("shows the no-results state when search returns empty", () => {
        // Override the search mock to return empty results.
        cy.visit("/", {
            onBeforeLoad(win) {
                setupTauriMock(win, {
                    search_content: () => ({
                        results: [],
                        page: 1,
                        has_more: false,
                        total_estimate: 0,
                    }),
                });
            },
        });

        cy.get(INPUT).type("no results query{enter}");

        cy.contains("No results found").should("be.visible");
    });

    it("does nothing when submitting an empty input", () => {
        // Submit on empty input is a no-op (handleSubmit guards on
        // !displayValue.trim()). No results panel should render.
        cy.get(INPUT).focus().type("{enter}");
        cy.contains("Test Video One").should("not.exist");
    });
});
