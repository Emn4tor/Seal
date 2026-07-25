import { useState } from "react";
import { api } from "../lib/tauri";
import type { Attachment, ExifField } from "../lib/types";

interface ImageLightboxProps {
  attachment: Attachment;
  onClose: () => void;
}

/** Full-size image viewer. When the image wasn't sent with EXIF stripped,
 * offers a "View EXIF" panel — since the sender chose to keep it, the
 * recipient should be able to see exactly what's in there too. */
export function ImageLightbox({ attachment, onClose }: ImageLightboxProps) {
  const [showExif, setShowExif] = useState(false);
  const [exifFields, setExifFields] = useState<ExifField[] | null>(null);
  const [loadingExif, setLoadingExif] = useState(false);
  const [exifError, setExifError] = useState<string | null>(null);

  async function handleViewExif() {
    setShowExif(true);
    if (exifFields !== null) return;
    setLoadingExif(true);
    setExifError(null);
    try {
      setExifFields(await api.getImageExif(attachment.data_base64));
    } catch (err) {
      setExifError(String(err));
    } finally {
      setLoadingExif(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex flex-col bg-ink/95 backdrop-blur-sm" onClick={onClose}>
      <div className="flex items-center justify-between px-5 py-3.5">
        <p className="truncate text-sm font-medium text-text">{attachment.filename}</p>
        <div className="flex items-center gap-2" onClick={(e) => e.stopPropagation()}>
          {!attachment.exif_stripped && (
            <button
              onClick={handleViewExif}
              className="rounded-md border border-border px-3 py-1.5 text-xs font-medium text-brass hover:border-brass-dim hover:bg-brass-wash"
            >
              View EXIF
            </button>
          )}
          <button
            onClick={onClose}
            aria-label="Close"
            className="flex h-8 w-8 items-center justify-center rounded-md text-text-muted hover:bg-surface hover:text-text"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
              <path d="m6 6 12 12M18 6 6 18" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
            </svg>
          </button>
        </div>
      </div>

      <div className="flex flex-1 items-center justify-center overflow-hidden px-6 pb-6">
        <img
          src={`data:${attachment.mime_type};base64,${attachment.data_base64}`}
          alt={attachment.filename}
          className="max-h-full max-w-full rounded-md object-contain"
          onClick={(e) => e.stopPropagation()}
        />
      </div>

      {showExif && (
        <div
          className="absolute inset-x-0 bottom-0 max-h-[45%] overflow-y-auto border-t border-border bg-surface px-5 py-4"
          onClick={(e) => e.stopPropagation()}
        >
          <div className="mb-2 flex items-center justify-between">
            <h3 className="font-display text-sm font-semibold text-text">Photo metadata</h3>
            <button onClick={() => setShowExif(false)} className="text-xs text-text-muted hover:text-text">
              Close
            </button>
          </div>
          {loadingExif && <p className="text-sm text-text-faint">Reading…</p>}
          {exifError && <p className="text-sm text-danger">{exifError}</p>}
          {exifFields && exifFields.length === 0 && (
            <p className="text-sm text-text-faint">No metadata found in this image.</p>
          )}
          {exifFields && exifFields.length > 0 && (
            <table className="w-full text-sm">
              <tbody>
                {exifFields.map((f, i) => (
                  <tr key={i} className="border-b border-border/50 last:border-0">
                    <td className="py-1.5 pr-4 align-top font-medium text-text-muted">{f.tag}</td>
                    <td className="py-1.5 text-text">{f.value}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      )}
    </div>
  );
}
