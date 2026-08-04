import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { ChatPane } from "./ChatPane";

/** Regression test for a real bug fixed this session: `handleSend` used to
 * clear the draft/attachment before `onSend` resolved, with no `catch` —
 * a failed send silently lost whatever you'd typed. See ChatPane.tsx's
 * `handleSend` for the fix (restore on failure + show an inline error). */
describe("ChatPane send failure", () => {
  it("restores the draft and shows an error when onSend rejects, and clears normally once it succeeds", async () => {
    const onSend = vi.fn().mockRejectedValueOnce(new Error("peer unreachable")).mockResolvedValueOnce(undefined);

    render(
      <ChatPane
        title="Test"
        subtitle="subtitle"
        sealStatus="secure"
        messages={[]}
        currentUserId="me"
        placeholder="No messages yet."
        onSend={onSend}
      />,
    );

    const input = screen.getByPlaceholderText("Write a sealed message…") as HTMLInputElement;
    const sendButton = screen.getByRole("button", { name: "Send" });

    fireEvent.change(input, { target: { value: "hello there" } });
    fireEvent.click(sendButton);

    // Failed send: the draft comes back, and an error is shown — never
    // silently disappears.
    await waitFor(() => expect(screen.getByText(/peer unreachable/)).toBeInTheDocument());
    expect(input.value).toBe("hello there");
    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith("hello there", null);

    // Retrying the same send now succeeds: composer clears, error goes away.
    fireEvent.click(sendButton);
    await waitFor(() => expect(onSend).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(input.value).toBe(""));
    expect(screen.queryByText(/peer unreachable/)).not.toBeInTheDocument();
  });
});
