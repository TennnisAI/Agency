import { useEffect, useState } from "react";
import {
  GIT_IDENTITY_EVENT,
  GIT_IDENTITY_SET_EVENT,
  GitIdentityOffer,
} from "../lib/gitIdentity";
import { toastSuccess } from "../lib/toast";
import GitIdentityDialog from "./GitIdentityDialog";

/**
 * Mounted once at the shell root (and once under onboarding): listens for
 * `offerGitIdentity` and shows the form. On save it announces
 * `agency:git-identity-set` with the repo path, so whatever surface hit the
 * missing-identity error can retry the commit it was mid-way through. Marks
 * itself on `window` so the offer knows a host is listening.
 */
export default function GitIdentityHost() {
  const [offer, setOffer] = useState<GitIdentityOffer | null>(null);

  useEffect(() => {
    window.__agencyGitIdentityHost = (window.__agencyGitIdentityHost ?? 0) + 1;
    const onOffer = (e: Event) => setOffer((e as CustomEvent<GitIdentityOffer>).detail);
    window.addEventListener(GIT_IDENTITY_EVENT, onOffer);
    return () => {
      window.removeEventListener(GIT_IDENTITY_EVENT, onOffer);
      window.__agencyGitIdentityHost = Math.max(0, (window.__agencyGitIdentityHost ?? 1) - 1);
    };
  }, []);

  if (!offer) return null;
  return (
    <GitIdentityDialog
      repoPath={offer.repoPath}
      reason={offer.reason}
      onSaved={() => {
        const { repoPath } = offer;
        setOffer(null);
        toastSuccess("Git identity saved");
        window.dispatchEvent(
          new CustomEvent(GIT_IDENTITY_SET_EVENT, { detail: { repoPath } }),
        );
      }}
      onClose={() => setOffer(null)}
    />
  );
}
