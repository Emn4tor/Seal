import { useEffect, useRef } from "react";
import splashSrc from "../assets/splash.mp4";
import { playBootTone } from "../lib/bootSound";

interface SplashProps {
  onFinished: () => void;
}

/** The boot splash: an externally-rendered clip (not a CSS animation) of
 * the CipherSeal mark spinning up and locking into its "secure" state.
 * Shown once per launch for a fixed ~3.0s regardless of the real boot
 * sequence underneath (see `App.tsx`'s `splashDone` gate).
 *
 * The clip's embedded audio stays muted bc its damn annoying; two synthesized tones
 * (`lib/bootSound.ts`) play instead, timed to the mark forming (~0.55s)
 * and the flash before it locks in (~1.85s), both well before the clip
 * ends at 3.0s.
 *
 * Falls back hard on anything going wrong: a load error finishes
 * immediately, and a backstop timer covers the rare case where `ended`
 * never fires — nothing here is allowed to hang the boot screen. */
export function Splash({ onFinished }: SplashProps) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const onFinishedRef = useRef(onFinished);
  onFinishedRef.current = onFinished;

  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;

    let done = false;
    function finish() {
      if (done) return;
      done = true;
      onFinishedRef.current();
    }

    const timeoutId = setTimeout(finish, 3800);
    let toneOneId: ReturnType<typeof setTimeout> | undefined;
    let toneTwoId: ReturnType<typeof setTimeout> | undefined;

    // Anchored to when playback actually starts, not to mount — scheduling
    // off the "playing" event keeps the tones in sync with the clip's own
    // beats regardless of how long it took to become playable.
    function scheduleTones() {
      toneOneId = setTimeout(() => playBootTone("form"), 550);
      toneTwoId = setTimeout(() => playBootTone("lock"), 1850);
    }

    video.addEventListener("ended", finish);
    video.addEventListener("error", finish);
    video.addEventListener("playing", scheduleTones, { once: true });

    return () => {
      clearTimeout(timeoutId);
      clearTimeout(toneOneId);
      clearTimeout(toneTwoId);
      video.removeEventListener("ended", finish);
      video.removeEventListener("error", finish);
      video.removeEventListener("playing", scheduleTones);
    };
  }, []);

  return (
    <div className="flex h-screen items-center justify-center bg-ink">
      <video
        ref={videoRef}
        src={splashSrc}
        className="h-[220px] w-[220px]"
        autoPlay
        muted
        playsInline
        preload="auto"
      />
    </div>
  );
}
