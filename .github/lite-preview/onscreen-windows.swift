// Prints "<width>x<height>" for every on-screen window owned by the given pid,
// one per line. Owner pid and bounds are readable without the Screen Recording
// permission (window titles are not), which is all the smoke needs: the splash
// is 360x360, the main window 1280x800.
//
// Usage: onscreen-windows <pid>
import CoreGraphics
import Foundation

guard CommandLine.arguments.count == 2, let pid = Int(CommandLine.arguments[1]) else {
  FileHandle.standardError.write("usage: onscreen-windows <pid>\n".data(using: .utf8)!)
  exit(2)
}

let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
let windows = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] ?? []
for window in windows {
  guard (window[kCGWindowOwnerPID as String] as? Int) == pid,
    let bounds = window[kCGWindowBounds as String] as? [String: Any],
    let width = bounds["Width"] as? Double,
    let height = bounds["Height"] as? Double
  else { continue }
  print("\(Int(width))x\(Int(height))")
}
