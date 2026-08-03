import { useEffect, useRef } from "react";
import splashSrc from "../assets/splash.mp4";

interface SplashProps {
  onFinished: () => void;
}

/** The boot splash: an externally-rendered clip (not a CSS animation) of
 * the CipherSeal mark spinning up and locking into its "secure" state, with
 * a synthesized whoosh + chime. Shown once per launch for a fixed ~1.8s
 * regardless of how long the real boot sequence takes underneath it — see
 * `App.tsx`'s `splashDone` gate.
 *
 * Falls back hard on anything going wrong: most webviews block autoplay
 * with sound before any user gesture has happened, so it starts muted and
 * only tries to unmute once actual playback has begun (never both
 * autoplaying *and* manually calling `play()` at once — racing the two is
 * what caused this to hang the whole page during development); a load
 * error finishes immediately; and a backstop timer covers the rare case
 * where `ended` never fires. After last session's "stuck on Waking up"
 * bug, nothing here is allowed to be able to hang the boot screen. */
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

    const timeoutId = setTimeout(finish, 2600);

    function tryUnmute() {
      if (video && video.muted && !video.paused) {
        video.muted = false;
      }
    }

    video.addEventListener("ended", finish);
    video.addEventListener("error", finish);
    // Only unmute after real playback has actually started — attempting a
    // second, competing `play()` call to "upgrade" to sound is what caused
    // the hang this replaced.
    video.addEventListener("playing", tryUnmute, { once: true });

    return () => {
      clearTimeout(timeoutId);
      video.removeEventListener("ended", finish);
      video.removeEventListener("error", finish);
      video.removeEventListener("playing", tryUnmute);
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
