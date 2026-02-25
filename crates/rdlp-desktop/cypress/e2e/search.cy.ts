// E2E tests for the search flow.
//
// Covers: typing a query, selecting a site, submitting, viewing results,
// paginating to the next page, and clearing the search.

import { setupTauriMock } from "../support/e2e";

describe("Search flow", () => {
    it("shows the search input on app load", () => {
        cy.get('input[placeholder="Search videos..."]').should("be.visible");
    });

    it("auto-selects the first search provider", () => {
        // The site selector should display the first provider from the mock
        cy.get('[aria-label="Search site"]').should("be.visible");
        // The SelectTrigger text should contain the first provider name after providers load
        cy.contains("RedTube").should("exist");
    });

    it("shows search results after submitting a query", () => {
        cy.get('input[placeholder="Search videos..."]').type("test query");
        cy.get('button[aria-label="Search"]').click();

        cy.contains("Test Video One").should("be.visible");
        cy.contains("Test Video Two").should("be.visible");
    });

    it("shows 'Load more results' button when there are more pages", () => {
        cy.get('input[placeholder="Search videos..."]').type("test query");
        cy.get('button[aria-label="Search"]').click();

        cy.contains("Load more results").should("be.visible");
    });

    it("loads the next page when 'Load more results' is clicked", () => {
        cy.get('input[placeholder="Search videos..."]').type("test query");
        cy.get('button[aria-label="Search"]').click();

        cy.contains("Test Video One").should("be.visible");
        cy.contains("Load more results").click();

        cy.contains("Test Video Three").should("be.visible");
        cy.contains("All results loaded").should("be.visible");
    });

    it("announces result count to screen readers", () => {
        cy.get('input[placeholder="Search videos..."]').type("test query");
        cy.get('button[aria-label="Search"]').click();

        cy.get('[aria-live="polite"]').should("contain", "results loaded");
    });

    it("clears the search when the clear button is clicked", () => {
        cy.get('input[placeholder="Search videos..."]').type("test query");

        // The clear (X) button appears when there is text in the input
        cy.get('button[aria-label="Clear search"]').should("be.visible").click();

        cy.get('input[placeholder="Search videos..."]').should("have.value", "");
    });

    it("shows 'No results' state when search returns empty", () => {
        // Override the search mock to return empty results
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

        cy.get('input[placeholder="Search videos..."]').type("no results query");
        cy.get('button[aria-label="Search"]').click();

        cy.contains("No results for").should("be.visible");
    });

    it("disables search button when input is empty", () => {
        cy.get('button[aria-label="Search"]').should("be.disabled");
    });

    it("enables search button after typing a query", () => {
        cy.get('input[placeholder="Search videos..."]').type("hello");
        cy.get('button[aria-label="Search"]').should("not.be.disabled");
    });
});
