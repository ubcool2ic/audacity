#!/usr/bin/env python3
"""BizyBot Extra-menu items 46-67 (docs/SPLIT_extra.md) vs Audacity 3.7.3 over mod-script-pipe.
Reuses compare_extra.py's harness; only adds the selection/cursor cases."""
import sys, compare_extra as X
X.SEL_CASES[:] = [
    (2.0, 4.0, "SelSetExtRight"), (3.0, 3.0, "SelSetExtRight"), (9.99, 10.0, "SelSetExtRight"),
    (2.0, 4.0, "SelCntrLeft"), (2.0, 4.0, "SelCntrRight"), (3.0, 3.0, "SelCntrLeft"),
    (2.0, 4.0, "CursorLeft"), (2.0, 4.0, "CursorRight"), (3.0, 3.0, "CursorLeft"),
    (3.0, 3.0, "CursorRight"), (0.0, 0.0, "CursorLeft"),
]
if __name__ == "__main__":
    sys.exit(X.main())
