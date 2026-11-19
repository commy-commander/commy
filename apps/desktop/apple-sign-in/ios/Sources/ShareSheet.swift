import Tauri
import UIKit

class ShareRequest: Decodable {
  let url: String
}

struct ShareResult: Encodable {
  let shared: Bool
}

extension AppleSignInPlugin {
  @objc public func shareLink(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ShareRequest.self)
    guard let link = URL(string: args.url), link.scheme == "https" || link.scheme == "commy" else {
      invoke.reject("That isn't a link Commy can share")
      return
    }
    DispatchQueue.main.async {
      guard let host = AppleSignInPlugin.frontScreen() else {
        invoke.reject("The share menu could not open")
        return
      }
      var answered = false
      let menu = UIActivityViewController(activityItems: [link], applicationActivities: nil)
      menu.completionWithItemsHandler = { _, completed, _, _ in
        if answered { return }
        answered = true
        invoke.resolve(ShareResult(shared: completed))
      }
      if let popover = menu.popoverPresentationController {
        popover.sourceView = host.view
        popover.sourceRect = CGRect(x: host.view.bounds.midX, y: host.view.bounds.maxY - 1, width: 1, height: 1)
        popover.permittedArrowDirections = []
      }
      host.present(menu, animated: true)
    }
  }

  private static func frontScreen() -> UIViewController? {
    let windows = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap { $0.windows }
    var front = (windows.first { $0.isKeyWindow } ?? windows.first)?.rootViewController
    while let shown = front?.presentedViewController, !shown.isBeingDismissed {
      front = shown
    }
    return front
  }
}
