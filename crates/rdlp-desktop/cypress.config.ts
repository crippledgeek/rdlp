import { defineConfig } from "cypress";

export default defineConfig({
    e2e: {
        baseUrl: "http://localhost:5173",
        specPattern: "cypress/e2e/**/*.cy.ts",
        supportFile: "cypress/support/e2e.ts",
        viewportWidth: 1280,
        viewportHeight: 720,
        video: false,
        screenshotOnRunFailure: false,
        setupNodeEvents(on) {
            on("task", {
                log(message: string) {
                    // eslint-disable-next-line no-console
                    console.log(message);
                    return null;
                },
            });
        },
    },
});
