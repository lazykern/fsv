# Attachment labels in detail views

CLI details and the main TUI share `pages.main_lines` / `conversation_lines`.
Their human-readable rendering removes a whole body line only when its visible
text is exactly an attached filename (also after the HTML list bullet). Native
HTML links have already become their visible labels at this stage. The canonical
paperclip entry stays, including files that are absent from the body text.
Sentences mentioning a filename and similarly named non-attachments stay intact.

This applies to planning sections, conversations and main-description attachment
lists. The change is render-only: source text, attachment arrays and JSON output
are unchanged. Offline tests exercise rich-text links, repeated filename lines,
attachments absent from the text, CLI output, and a real TUI Pilot detail pane.
