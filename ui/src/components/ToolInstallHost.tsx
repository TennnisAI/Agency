import { useEffect, useState } from "react";
import { MISSING_TOOL_EVENT, MissingToolDetail } from "../lib/missingTool";
import ToolInstallDialog from "./ToolInstallDialog";

/**
 * Mounted once at the shell root (and once under onboarding): listens for
 * `offerToolInstall` and shows the dialog. Marks itself on `window` so the
 * offer can tell whether anyone is listening and fall back to a toast.
 */
export default function ToolInstallHost() {
  const [offer, setOffer] = useState<MissingToolDetail | null>(null);

  useEffect(() => {
    window.__agencyToolHost = (window.__agencyToolHost ?? 0) + 1;
    const onOffer = (e: Event) => setOffer((e as CustomEvent<MissingToolDetail>).detail);
    window.addEventListener(MISSING_TOOL_EVENT, onOffer);
    return () => {
      window.removeEventListener(MISSING_TOOL_EVENT, onOffer);
      window.__agencyToolHost = Math.max(0, (window.__agencyToolHost ?? 1) - 1);
    };
  }, []);

  if (!offer) return null;
  return <ToolInstallDialog tool={offer.tool} reason={offer.reason} onClose={() => setOffer(null)} />;
}
