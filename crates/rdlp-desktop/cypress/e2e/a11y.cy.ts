// Automated WCAG 2.1 AA accessibility regression spec.
//
// Runs axe-core (via cypress-axe) against every primary view. Per-page known
// violations are allowlisted in cypress/support/a11y-allowlist.ts and shrink
// commit-by-commit during the a11y sprint.
//
// Targeted ARIA attribute assertions will be added by subsequent commits in
// this sprint alongside axe checks because missing aria-expanded /
// aria-pressed / aria-live semantics are not reliably surfaced by axe-core's
// default rule set.

import { allowlistFor } from "../support/a11y-allowlist";

const WCAG_AA_TAGS = ["wcag2a", "wcag2aa", "wcag21aa"];

function injectAndCheck(page: Parameters<typeof allowlistFor>[0]) {
    cy.injectAxe();
    cy.checkA11y(undefined, {
        runOnly: { type: "tag", values: WCAG_AA_TAGS },
        rules: allowlistFor(page),
    });
}

describe("a11y: WCAG 2.1 AA regression", () => {
    beforeEach(() => {
        cy.visit("/");
    });

    it("Search view (empty)", () => {
        cy.goToSearch();
        injectAndCheck("search-empty");
    });

    it("Queue view (empty)", () => {
        cy.goToQueue();
        injectAndCheck("queue-empty");
    });

    it("History view", () => {
        cy.goToHistory();
        injectAndCheck("history");
    });

    it("Settings view", () => {
        cy.goToSettings();
        injectAndCheck("settings");
    });

    it("Logs drawer expanded", () => {
        cy.get('[aria-label="Expand drawer"], [aria-label="Collapse drawer"]')
            .first()
            .click();
        injectAndCheck("logs-expanded");
    });

    it("drawer toggle exposes aria-expanded and aria-controls", () => {
        cy.get('[aria-label="Expand drawer"]').as("toggle");
        cy.get("@toggle")
            .should("have.attr", "aria-expanded", "false")
            .and("have.attr", "aria-controls", "bottom-drawer-panel");
        cy.get("#bottom-drawer-panel").should("exist");
        cy.get("@toggle").click();
        cy.get('[aria-label="Collapse drawer"]')
            .should("have.attr", "aria-expanded", "true")
            .and("have.attr", "aria-controls", "bottom-drawer-panel");
    });
});
