# Smart stretch prototype (throwaway)

Answers the ticket "How should Smart stretch look and be tuned?". Open `index.html` in a browser; ←/→ or the bottom bar switches variants (`?variant=A|B|C|D`, `?ratio=21:9`). Drop any 16:9 screenshot onto the page to judge real content.

- **A Three strips**: undistorted centre strip + evenly stretched sides (visible seams).
- **B Smooth curve**: `g(v) = (1−t)·v + t·(k·v − (k−1)·v^p)`, with `p = min(3, 0.95·k/(k−1))` for k > 1 (keeps it monotonic), `p = 3` for k < 1. `t` = strength.
- **C Zoom + curve**: crop top/bottom by zoom z (1..k), then B at full strength on the remaining k/z.
- **D Plain Stretch**: baseline.

**Verdict:** Smart stretch = **B** with one strength slider (0% = plain Stretch, 100% = undistorted centre, default 50%); narrower Target ratios squeeze the edges with the same curve.
