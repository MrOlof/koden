import { activeLeafId, runCommand, submitAndExpect, waitForBus } from "./_helpers";

// MVP scenario (a): drive a terminal end-to-end through the bus.
// Palette command -> new terminal tab -> submit a command -> assert the buffer.
describe("terminal: type and read back", () => {
  before(waitForBus);

  it("opens a terminal tab and echoes a sentinel into the buffer", async () => {
    await runCommand("tab.new");

    const leafId = await activeLeafId();
    expect(leafId).not.toBe(null);

    await submitAndExpect(leafId as number, "echo koden-sentinel-9271", "koden-sentinel-9271");
  });
});
