# Settings drawer prototype (throwaway)

Answers the ticket "How is the settings panel summoned and laid out over the fullscreen picture?". Open `index.html` in a browser; ←/→ or the bottom bar switches layouts (`?variant=A|B|C`, `&open=1` to start open). The box is a 21:9 monitor. Fill mode, Target ratio, Smart stretch strength, Picture adjustments and drawer side are live; the rest are stubs.

- **A** Tabs on a side rail (Source / Display / Colour / Audio), one section at a time.
- **B** One scrolling list with collapsible sections.
- **C** Quick panel (Fill mode, strength, ratio, volume) + "All settings" full view.

**Verdict:** A. The drawer opens with Tab, right-click on the picture, or the edge handle; Esc closes. Everything applies live except a Source or Video format change (brief restart). First run uses a separate Welcome screen.
