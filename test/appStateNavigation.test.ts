import { describe, expect, it } from "vitest";
import { createAppSuspendNavigationCoordinator } from "../src/appStateNavigation";

describe("createAppSuspendNavigationCoordinator", () => {
  it("navigates immediately when the active App has no state adapter", () => {
    const events: string[] = [];
    const coordinator = createAppSuspendNavigationCoordinator();

    coordinator.run(null, () => events.push("navigate"));

    expect(events).toEqual(["navigate"]);
  });

  it("waits for state capture before navigating", async () => {
    const events: string[] = [];
    let finishSave = () => {};
    const coordinator = createAppSuspendNavigationCoordinator();

    coordinator.run(
      () => new Promise<void>((resolve) => {
        finishSave = () => {
          events.push("saved");
          resolve();
        };
      }),
      () => events.push("navigate"),
    );

    await Promise.resolve();
    expect(events).toEqual([]);
    finishSave();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(events).toEqual(["saved", "navigate"]);
  });

  it("uses the latest destination when navigation changes during capture", async () => {
    const events: string[] = [];
    let finishSave = () => {};
    const coordinator = createAppSuspendNavigationCoordinator();
    const saveState = () => new Promise<void>((resolve) => {
      finishSave = resolve;
    });

    coordinator.run(saveState, () => events.push("project"));
    coordinator.run(saveState, () => events.push("settings"));
    await Promise.resolve();
    finishSave();
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(events).toEqual(["settings"]);
  });
});
