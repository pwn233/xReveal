// Says whether this process (and so the terminal that started it) may post synthetic input events.
// macOS asks for this under Privacy & Security › Accessibility.
import ApplicationServices
import CoreGraphics

print("AXIsProcessTrusted:", AXIsProcessTrusted())
print("CGPreflightPostEventAccess:", CGPreflightPostEventAccess())
