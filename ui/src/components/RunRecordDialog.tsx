import { useEffect, useState } from "react";
import { RunInfo, RunConversation, readRunRecord, readRunConversation } from "../api";
import { useModalKeys } from "../hooks/useModalKeys";
import { fmtStamp } from "../lib/issues";
import ModalBackdrop from "./ModalBackdrop";
import Markdown from "./Markdown";

/**
 * What an archived agent left behind: its record, and its conversation.
 *
 * The record is the answer to the part of archiving that used to need the
 * branch: a merged run's branch is deleted with its worktree, and this is
 * where "what did that agent actually do" now lives. The file is markdown in
 * the project's own `.agency/records/`, so it outlives Agency and can be read
 * without it; this dialog is a convenience, not the only way in.
 *
 * The conversation is the other half of the same question, "what did it say",
 * rendered read-only from the transcript the archive rescued (AGE-152). Only
 * two agents' transcript formats have been read for real, so for the rest
 * this tab says it cannot see the conversation, which is a different claim
 * from there not being one.
 */
export default function RunRecordDialog({
  run,
  onClose,
}: {
  run: RunInfo;
  onClose: () => void;
}) {
  // A run archived before records existed can still have a conversation, and
  // opening it on an empty record pane would read as "nothing here".
  const [tab, setTab] = useState<"record" | "conversation">(
    run.archived && !run.archived.hasRecord && run.archived.hasConversation
      ? "conversation"
      : "record",
  );
  const [text, setText] = useState<string | null>(null);
  const [convo, setConvo] = useState<RunConversation | null>(null);
  const [error, setError] = useState("");
  useModalKeys(onClose, true);

  useEffect(() => {
    let live = true;
    readRunRecord(run.id)
      .then((t) => live && setText(t ?? ""))
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, [run.id]);

  // Fetched only once the tab is opened: the transcript can be megabytes of
  // JSONL, and most visits are for the record.
  useEffect(() => {
    if (tab !== "conversation" || convo) return;
    let live = true;
    readRunConversation(run.id)
      .then((c) => live && setConvo(c))
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, [tab, convo, run.id]);

  const started = (iso: string | null) => {
    const ms = iso ? Date.parse(iso) : NaN;
    return Number.isNaN(ms) ? null : fmtStamp(ms / 1000);
  };

  return (
    <ModalBackdrop onBackdropClick={onClose}>
      <div
        className="modal run-record"
        role="dialog"
        aria-modal="true"
        aria-label="Run record"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3>{run.title || run.branch}</h3>
          <div className="seg run-record-seg">
            <button className={tab === "record" ? "on" : ""} onClick={() => setTab("record")}>
              Record
            </button>
            <button
              className={tab === "conversation" ? "on" : ""}
              onClick={() => setTab("conversation")}
            >
              Conversation
            </button>
          </div>
          <button className="modal-x" onClick={onClose}>✕</button>
        </div>
        <div className="modal-body run-record-body">
          {error ? (
            <p className="git-error">{error}</p>
          ) : tab === "record" ? (
            text === null ? (
              <p>Reading…</p>
            ) : text === "" ? (
              // A run archived before records existed, or one whose project
              // folder has moved. Say which is missing rather than showing an
              // empty pane that reads as a failure.
              <p>
                No record for this run. It was archived before Agency kept them, or the project
                folder has moved since.
              </p>
            ) : (
              <Markdown text={text} />
            )
          ) : convo === null ? (
            <p>Reading…</p>
          ) : !convo.supported ? (
            // "We cannot see this" and "the agent said nothing" are different
            // claims, and only this one is true here.
            <p>
              Agency cannot read {run.agent} transcripts, so the conversation cannot be shown.
              That is a limit of Agency's, not a sign the agent said nothing.
            </p>
          ) : convo.sessions.length === 0 ? (
            run.worktree ? (
              <p>
                No conversation is kept for this run. It may have been archived before Agency
                rescued transcripts, or the agent never wrote one.
              </p>
            ) : (
              // A checkout run's sessions live in the agent's store for the
              // project folder itself, beside every other conversation the
              // user had there; none of it can be attributed to this run.
              <p>
                This agent worked in the project's own folder, so its conversation sits in{" "}
                {run.agent}'s store beside your other sessions there, and Agency cannot tell
                which of them was this run's. The record names the directory.
              </p>
            )
          ) : (
            <div className="convo">
              {convo.sessions.map((s, i) => (
                <div className="convo-session" key={i}>
                  {(convo.sessions.length > 1 || s.title) && (
                    <div className="convo-session-head">
                      {convo.sessions.length > 1 ? `Session ${i + 1}` : "Session"}
                      {s.title ? ` · ${s.title}` : ""}
                      {started(s.started) ? ` · ${started(s.started)}` : ""}
                    </div>
                  )}
                  {s.turns.map((t, j) =>
                    t.role === "tool" ? (
                      <div className="convo-tool" key={j}>
                        <span aria-hidden>▸</span> {t.text}
                      </div>
                    ) : t.role === "user" ? (
                      <div className="convo-user" key={j}>
                        {t.text}
                      </div>
                    ) : (
                      <Markdown className="convo-assistant" text={t.text} key={j} />
                    ),
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </ModalBackdrop>
  );
}
