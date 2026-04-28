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

function logViolations(violations: { id: string; impact?: string; description: string; nodes: { target: unknown[] }[] }[]) {
    if (violations.length === 0) return;
    const data = violations.map((v) => ({
        rule: v.id,
        impact: v.impact ?? "n/a",
        nodes: v.nodes.length,
        firstSelector: JSON.stringify(v.nodes[0]?.target),
        description: v.description.slice(0, 100),
    }));
    // eslint-disable-next-line no-console
    console.table(data);
    cy.task("log", JSON.stringify(data, null, 2)).then(() => {});
}

function injectAndCheck(page: Parameters<typeof allowlistFor>[0]) {
    cy.injectAxe();
    cy.checkA11y(
        undefined,
        {
            runOnly: { type: "tag", values: WCAG_AA_TAGS },
            rules: allowlistFor(page),
        },
        logViolations,
    );
}

describe("a11y: WCAG 2.1 AA regression", () => {
    // Page-load + Tauri IPC mock is set up by the global beforeEach in
    // cypress/support/e2e.ts. Do not call cy.visit() here — it would
    // re-visit without the onBeforeLoad mock injection.

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
        // Ensure drawer is expanded regardless of state inherited from prior tests.
        cy.get("body").then(($body) => {
            if ($body.find('[aria-label="Expand drawer"]').length > 0) {
                cy.get('[aria-label="Expand drawer"]').click();
            }
        });
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

    it("log severity chips toggle aria-pressed", () => {
        // Ensure drawer is expanded regardless of state inherited from prior tests.
        cy.get("body").then(($body) => {
            if ($body.find('[aria-label="Expand drawer"]').length > 0) {
                cy.get('[aria-label="Expand drawer"]').click();
            }
        });
        // All four chips start pressed (default filter set is full).
        ["info", "warn", "error", "debug"].forEach((level) => {
            cy.contains("button", new RegExp(`^${level}$`, "i"))
                .should("have.attr", "aria-pressed", "true");
        });
        // Toggle "debug" off and re-check. The chip is sometimes occluded by
        // the focus-ring overlay in headless runs; force the click since we
        // only care about the aria-pressed state flip.
        cy.contains("button", /^debug$/i).click({ force: true });
        cy.contains("button", /^debug$/i)
            .should("have.attr", "aria-pressed", "false");
    });

    it("status pill and empty states use live regions", () => {
        // Drawer status row announces job state.
        cy.get('[data-testid="drawer-status"]').should(
            "have.attr",
            "aria-live",
            "polite",
        );
        // Analyze empty state lives inside a polite live region.
        cy.goToSearch();
        cy.contains("Paste a URL to begin")
            .closest('[aria-live="polite"]')
            .should("exist");
        // StatusBadge defaults to role=status (polite). The 'failed' variant
        // upgrades to role=alert.
        cy.goToQueue();
        cy.get('[role="status"], [role="alert"]').should("exist");
    });
});
