// Custom Cypress commands for rdlp-desktop E2E tests.
//
// Commands added here are available as cy.commandName() in all tests.

/// <reference types="cypress" />

import { setupTauriMock } from "./tauriMock";

declare global {
    namespace Cypress {
        interface Chainable {
            /**
             * Type a search query into the search input and submit.
             * @param query - The search query string.
             */
            search(query: string): Chainable<void>;

            /**
             * Navigate to the Settings tab in the sidebar.
             */
            goToSettings(): Chainable<void>;

            /**
             * Navigate to the Search tab in the sidebar.
             */
            goToSearch(): Chainable<void>;

            /**
             * Navigate to the Queue tab in the sidebar.
             */
            goToQueue(): Chainable<void>;

            /**
             * Navigate to the History tab in the sidebar.
             */
            goToHistory(): Chainable<void>;

            /**
             * Visit the app with the Tauri IPC mock installed, overriding
             * individual command handlers for this test.
             *
             * The global `beforeEach` in support/e2e.ts already visits with the
             * default handlers; use this only when a test needs different ones.
             * @param overrides - Handlers to replace, keyed by command name.
             */
            visitWithMock(
                overrides?: Record<string, (args: Record<string, unknown>) => unknown>,
            ): Chainable<void>;
        }
    }
}

Cypress.Commands.add("search", (query: string) => {
    cy.get('input[placeholder="Search videos..."]').clear().type(query);
    cy.get('button[aria-label="Search"]').click();
});

// IconRail nav buttons are icon-only with aria-label "<View> (Ctrl+N)";
// match by label prefix so the keybinding suffix can change without
// breaking these helpers.
Cypress.Commands.add("goToSettings", () => {
    cy.get('[aria-label^="Settings"]').first().click();
});

Cypress.Commands.add("goToSearch", () => {
    cy.get('[aria-label^="Analyze"]').first().click();
});

Cypress.Commands.add("goToQueue", () => {
    cy.get('[aria-label^="Queue"]').first().click();
});

Cypress.Commands.add("goToHistory", () => {
    cy.get('[aria-label^="History"]').first().click();
});

// Both specs that need custom handlers wrote this same `cy.visit("/", {
// onBeforeLoad(win) { setupTauriMock(win, ...) } })` block. It is also the
// form the unregistered-command error tells you to reach for — and that error,
// and the doc comment above `setupTauriMock`, both named `cy.visitWithMock()`
// while nothing defined it. Defining it once makes the citation true and gives
// the two call sites one mechanism to share.
Cypress.Commands.add(
    "visitWithMock",
    (overrides: Record<string, (args: Record<string, unknown>) => unknown> = {}) => {
        cy.visit("/", {
            onBeforeLoad(win) {
                setupTauriMock(win, overrides);
            },
        });
    },
);

// cypress-axe registers cy.injectAxe() and cy.checkA11y() globally via its
// import in cypress/support/e2e.ts. Types ship with the package; no extra
// declaration needed here.

export {};
