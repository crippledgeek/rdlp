// Custom Cypress commands for rdlp-desktop E2E tests.
//
// Commands added here are available as cy.commandName() in all tests.

/// <reference types="cypress" />

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

// cypress-axe registers cy.injectAxe() and cy.checkA11y() globally via its
// import in cypress/support/e2e.ts. Types ship with the package; no extra
// declaration needed here.

export {};
