// E2E tests for the Settings page.
//
// Covers: loading settings, changing values, saving, and error handling.

// Selectors specific to React Aria Components rendered by Jolly UI:
// - Checkbox renders as a <label> with data-selected reflecting checked state.
// - The Output Directory input has id="output-dir".

const OUTPUT_DIR_INPUT = "#output-dir";

function checkboxByLabel(label: string) {
    return cy.contains("label", label);
}

describe("Settings page", () => {
    beforeEach(() => {
        cy.goToSettings();
    });

    it("shows the Settings heading", () => {
        cy.contains("h2", "Settings").should("be.visible");
    });

    it("displays the output directory field", () => {
        cy.get('label').contains("Output Directory").should("be.visible");
        cy.get(OUTPUT_DIR_INPUT).should("have.value", "/home/user/Downloads");
    });

    it("shows the Browse button for output directory", () => {
        cy.contains("button", "Browse").should("be.visible");
    });

    it("shows the Save Settings button", () => {
        cy.contains("button", "Save Settings").scrollIntoView().should("be.visible");
    });

    it("shows the embed thumbnail checkbox checked by default", () => {
        checkboxByLabel("Embed thumbnails").should("have.attr", "data-selected", "true");
    });

    it("shows the embed metadata checkbox checked by default", () => {
        checkboxByLabel("Embed metadata").should("have.attr", "data-selected", "true");
    });

    it("shows the verbose logging checkbox unchecked by default", () => {
        checkboxByLabel("Verbose logging").should("not.have.attr", "data-selected");
    });

    it("toggles the verbose logging checkbox", () => {
        checkboxByLabel("Verbose logging").click();
        checkboxByLabel("Verbose logging").should("have.attr", "data-selected", "true");
    });

    it("saves settings without error on clicking Save Settings", () => {
        cy.contains("button", "Save Settings").scrollIntoView().click();

        // No error alert should appear
        cy.get('[role="alert"]').should("not.exist");
    });

    it("updates output directory when Browse is clicked and directory is returned", () => {
        cy.contains("button", "Browse").click();
        // The pick_directory mock returns "/home/user/Videos"
        cy.get(OUTPUT_DIR_INPUT).should("have.value", "/home/user/Videos");
    });

    it("shows error alert when save fails", () => {
        cy.visitWithMock({
            update_settings: () => {
                throw new Error("Disk full");
            },
        });

        cy.goToSettings();
        cy.contains("button", "Save Settings").scrollIntoView().click();

        cy.get('[role="alert"]').should("contain", "Disk full");
    });

    it("shows the Remux Format option in Format Defaults", () => {
        cy.contains("Remux Format").should("be.visible");
    });

    it("shows the Subtitle Format option in Format Defaults", () => {
        cy.contains("Subtitle Format").should("be.visible");
    });
});
