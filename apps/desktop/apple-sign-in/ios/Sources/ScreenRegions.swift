import UIKit
import WebKit

final class ScreenRegions: UIView {
  private weak var page: WKWebView?
  private var loading: NSKeyValueObservation?

  override class var requiresConstraintBasedLayout: Bool { true }

  nonisolated static func watch(_ page: WKWebView) {
    DispatchQueue.main.async {
      let watcher = ScreenRegions(frame: page.bounds)
      watcher.page = page
      watcher.isUserInteractionEnabled = false
      watcher.backgroundColor = .clear
      watcher.translatesAutoresizingMaskIntoConstraints = false
      page.addSubview(watcher)
      NSLayoutConstraint.activate([
        watcher.topAnchor.constraint(equalTo: page.topAnchor),
        watcher.bottomAnchor.constraint(equalTo: page.bottomAnchor),
        watcher.leadingAnchor.constraint(equalTo: page.leadingAnchor),
        watcher.trailingAnchor.constraint(equalTo: page.trailingAnchor),
      ])
      watcher.loading = page.observe(\.isLoading) { [weak watcher] _, _ in
        DispatchQueue.main.async { watcher?.tell() }
      }
      watcher.setNeedsUpdateConstraints()
    }
  }

  override func updateConstraints() {
    tell()
    super.updateConstraints()
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    tell()
  }

  private func covered() -> [CGRect] {
    if #available(iOS 27.1, *) {
      return reservedRegions(kind: .occlusion).map { $0.frame }
    }
    return []
  }

  private func tell() {
    guard let page = page, bounds.width > 0 else { return }
    let size = bounds.size
    let regions = covered().map { "[\(Int($0.minX.rounded(.down))),\(Int($0.minY.rounded(.down))),\(Int($0.width.rounded(.up))),\(Int($0.height.rounded(.up)))]" }
    let view = "{\"w\":\(Int(size.width)),\"h\":\(Int(size.height)),\"top\":\(Int(page.safeAreaInsets.top)),\"regions\":[\(regions.joined(separator: ","))]}"
    page.evaluateJavaScript("window.screenRegions=\(view);window.dispatchEvent(new Event('screenregions'));", completionHandler: nil)
  }
}
