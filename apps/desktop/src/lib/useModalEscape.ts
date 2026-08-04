import { useEffect } from "react";

/** Closes whatever modal/overlay called this on Escape — none of this
 * app's dialogs handled Escape before this, only an explicit click on a
 * close button or the backdrop. Attaches at the `document` level rather
 * than on a specific element so it works regardless of what currently has
 * focus inside the dialog. */
export function useModalEscape(onClose: () => void) {
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);
}
