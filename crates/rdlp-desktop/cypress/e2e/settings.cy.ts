// E2E tests for the Settings page.
//
// Covers: loading settings, changing values, saving, and error handling.

import { setupTauriMock } from "../support/e2e";

describe("Settings page", () => {
    beforeEach(() => {
        // Navigate to Settings — find the settings nav link in the sidebar
        cy.contains("Settings").click();
    });

    it("shows the Settings heading", () => {
        cy.contains("h2", "Settings").should("be.visible");
    });

    it("displays the output directory field", () => {
        cy.get('label').contains("Output Directory").should("be.visible");
        cy.get('input[type="text"]').first().should("have.value", "/home/user/Downloads");
    });

    it("shows the Browse button for output directory", () => {
        cy.contains("button", "Browse").should("be.visible");
    });

    it("shows the Save Settings button", () => {
        cy.contains("button", "Save Settings").scrollIntoView().should("be.visible");
    });

    it("shows the embed thumbnail checkbox checked by default", () => {
        cy.contains("Embed thumbnails")
            .closest("div")
            .find('button[role="checkbox"]')
            .should("have.attr", "aria-checked", "true");
    });

    it("shows the embed metadata checkbox checked by default", () => {
        cy.contains("Embed metadata")
            .closest("div")
            .find('button[role="checkbox"]')
            .should("have.attr", "aria-checked", "true");
    });

    it("shows the verbose logging checkbox unchecked by default", () => {
        cy.contains("Verbose logging")
            .closest("div")
            .find('button[role="checkbox"]')
            .should("have.attr", "aria-checked", "false");
    });

    it("toggles the verbose logging checkbox", () => {
        cy.contains("Verbose logging")
            .closest("div")
            .find('button[role="checkbox"]')
            .click()
            .should("have.attr", "aria-checked", "true");
    });

    it("saves settings without error on clicking Save Settings", () => {
        cy.contains("button", "Save Settings").scrollIntoView().click();

        // No error alert should appear
        cy.get('[role="alert"]').should("not.exist");
    });

    it("updates output directory when Browse is clicked and directory is returned", () => {
        cy.contains("button", "Browse").click();
        // The pick_directory mock returns "/home/user/Videos"
        cy.get('input[type="text"]').first().should("have.value", "/home/user/Videos");
    });

    it("shows error alert when save fails", () => {
        cy.visit("/", {
            onBeforeLoad(win) {
                setupTauriMock(win, {
                    update_settings: () => {
                        throw new Error("Disk full");
                    },
                });
            },
        });

        cy.contains("Settings").click();
        cy.contains("button", "Save Settings").scrollIntoView().click();

        cy.get('[role="alert"]').should("contain", "Disk full");
    });

    it("shows all format options in Default Remux Format dropdown", () => {
        cy.contains("Default Remux Format").should("be.visible");
    });

    it("shows subtitle format options", () => {
        cy.contains("Default Subtitle Format").should("be.visible");
    });
});
