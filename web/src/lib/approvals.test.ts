import { describe, expect, it } from "vitest";
import { answerSaid } from "@/lib/approvals";

describe("answerSaid", () => {
  it("is the words the chat keeps as said for each decision", () => {
    expect(answerSaid("once")).toBe("Allow once");
    expect(answerSaid("conversation")).toBe("Allow for this conversation");
    expect(answerSaid("deny")).toBe("Deny");
  });
});
