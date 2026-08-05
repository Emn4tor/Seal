export type SealStatus = "idle" | "connecting" | "secure";

interface CipherSealProps {
  status: SealStatus;
  size?: number;
  className?: string;
}

/**
 * The app's signature mark: concentric cipher-wheel rings that
 * counter-rotate while a session is establishing, and settle into a solid
 * verdigris center once it's locked in. Used as the connection-status
 * indicator, the verified-contact badge, and the onboarding hero mark.
 */
export function CipherSeal({ status, size = 20, className }: CipherSealProps) {
  const connecting = status === "connecting";
  const centerColor = status === "secure" ? "var(--color-verdigris)" : "var(--color-brass)";
  const innerColor = status === "secure" ? "var(--color-verdigris)" : "var(--color-brass-dim)";
  const dim = status === "idle" ? 0.35 : 0.9;

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
      className={className}
    >
      <g className={connecting ? "seal-ring-outer is-connecting" : "seal-ring-outer"} style={{ transformOrigin: "12px 12px" }}>
        <circle cx="12" cy="12" r="10" stroke="var(--color-brass)" strokeWidth="1.25" strokeDasharray="2 3" opacity={dim} />
      </g>
      <g className={connecting ? "seal-ring-inner is-connecting" : "seal-ring-inner"} style={{ transformOrigin: "12px 12px" }}>
        <circle cx="12" cy="12" r="6.5" stroke={innerColor} strokeWidth="1.25" strokeDasharray="1.5 2.5" opacity={dim} />
      </g>
      <circle
        cx="12"
        cy="12"
        r="2.25"
        fill={centerColor}
        opacity={status === "idle" ? 0.5 : 1}
        className={connecting ? "seal-center is-connecting" : "seal-center"}
        style={{ transformOrigin: "12px 12px" }}
      />
    </svg>
  );
}
