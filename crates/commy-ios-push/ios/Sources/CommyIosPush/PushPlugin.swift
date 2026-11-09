import Foundation
import ObjectiveC
import Tauri
import UIKit
import UserNotifications

struct TokenAnswer: Encodable {
  let token: String?
  let service: String
  let allowed: Bool
}

final class AppleTokens {
  static let shared = AppleTokens()

  private let waitLimit: TimeInterval = 20
  private var token: String?
  private var waiting: [(String?) -> Void] = []
  private var handlersAdded = false
  private var keptDelegate: UIApplicationDelegate?

  func ask(_ done: @escaping (String?) -> Void) {
    DispatchQueue.main.async {
      self.addDelegateHandlers()
      self.waiting.append(done)
      UIApplication.shared.registerForRemoteNotifications()
      DispatchQueue.main.asyncAfter(deadline: .now() + self.waitLimit) { self.finish(self.token) }
    }
  }

  private func finish(_ token: String?) {
    if let token = token { self.token = token }
    let callbacks = waiting
    waiting.removeAll()
    callbacks.forEach { $0(token) }
  }

  private func addDelegateHandlers() {
    guard !handlersAdded, let delegate = UIApplication.shared.delegate else { return }
    handlersAdded = true
    let delegateClass: AnyClass = type(of: delegate)
    let registered: @convention(block) (AnyObject, UIApplication, Data) -> Void = { _, _, data in
      let hex = data.map { String(format: "%02x", $0) }.joined()
      DispatchQueue.main.async { AppleTokens.shared.finish(hex) }
    }
    let failed: @convention(block) (AnyObject, UIApplication, NSError) -> Void = { _, _, error in
      Logger.info("Apple push registration failed: \(error.localizedDescription)")
      DispatchQueue.main.async { AppleTokens.shared.finish(nil) }
    }
    add(
      #selector(UIApplicationDelegate.application(_:didRegisterForRemoteNotificationsWithDeviceToken:)),
      imp_implementationWithBlock(registered), to: delegateClass)
    add(
      #selector(UIApplicationDelegate.application(_:didFailToRegisterForRemoteNotificationsWithError:)),
      imp_implementationWithBlock(failed), to: delegateClass)
    keptDelegate = delegate
    UIApplication.shared.delegate = nil
    UIApplication.shared.delegate = delegate
  }

  private func add(_ selector: Selector, _ implementation: IMP, to delegateClass: AnyClass) {
    if !class_addMethod(delegateClass, selector, implementation, "v@:@@"),
      let existing = class_getInstanceMethod(delegateClass, selector)
    {
      method_setImplementation(existing, implementation)
    }
  }
}

enum ApnsEnvironment {
  static func service() -> String {
    guard let path = Bundle.main.path(forResource: "embedded", ofType: "mobileprovision"),
      let data = FileManager.default.contents(atPath: path),
      let text = String(data: data, encoding: .isoLatin1)
    else { return "apns" }
    let squashed = text.components(separatedBy: .whitespacesAndNewlines).joined()
    return squashed.contains("<key>aps-environment</key><string>development</string>") ? "apns-dev" : "apns"
  }
}

class PushPlugin: Plugin {
  @objc public func token(_ invoke: Invoke) {
    let center = UNUserNotificationCenter.current()
    center.removeAllDeliveredNotifications()
    center.requestAuthorization(options: [.alert, .sound, .badge]) { _, _ in
      center.getNotificationSettings { settings in
        let allowed = settings.authorizationStatus == .authorized || settings.authorizationStatus == .provisional
        AppleTokens.shared.ask { token in
          invoke.resolve(TokenAnswer(token: token, service: ApnsEnvironment.service(), allowed: allowed))
        }
      }
    }
  }
}

@_cdecl("init_plugin_commy_push")
func initPlugin() -> Plugin {
  return PushPlugin()
}
