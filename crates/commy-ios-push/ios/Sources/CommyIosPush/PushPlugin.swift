import Foundation
import ObjectiveC
import Security
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

struct PreviewsArgs: Decodable {
  let json: String
}

enum SharedPreviews {
  static let service = "chat.commy.mobile.previews"
  static let account = "wake-previews"

  static func save(_ data: Data) -> Bool {
    let query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
      kSecAttrAccount as String: account,
    ]
    let values: [String: Any] = [
      kSecValueData as String: data,
      kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
    ]
    let status = SecItemUpdate(query as CFDictionary, values as CFDictionary)
    if status == errSecSuccess { return true }
    guard status == errSecItemNotFound else { return false }
    return SecItemAdd(query.merging(values) { _, new in new } as CFDictionary, nil) == errSecSuccess
  }
}

struct TargetAnswer: Encodable {
  let target: String?
}

final class NotificationTaps: NSObject, UNUserNotificationCenterDelegate {
  static let shared = NotificationTaps()

  private let lock = NSLock()
  private var target: String?
  private var earlier: UNUserNotificationCenterDelegate?

  func watch() {
    if Thread.isMainThread {
      becomeDelegate()
    } else {
      DispatchQueue.main.async { self.becomeDelegate() }
    }
  }

  func take() -> String? {
    lock.lock()
    defer { lock.unlock() }
    let taken = target
    target = nil
    return taken
  }

  private func keep(_ tapped: String?) {
    guard let tapped = tapped else { return }
    lock.lock()
    target = tapped
    lock.unlock()
  }

  private func becomeDelegate() {
    let center = UNUserNotificationCenter.current()
    if let current = center.delegate, current === self { return }
    earlier = center.delegate
    center.delegate = self
  }

  static func target(in info: [AnyHashable: Any]) -> String? {
    if let target = info["target"] as? String { return target }
    return (info["__EXTRA__"] as? [String: Any])?["target"] as? String
  }

  func userNotificationCenter(
    _ center: UNUserNotificationCenter, willPresent notification: UNNotification,
    withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
  ) {
    if earlier?.userNotificationCenter?(center, willPresent: notification, withCompletionHandler: completionHandler) == nil {
      completionHandler([])
    }
  }

  func userNotificationCenter(
    _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
    withCompletionHandler completionHandler: @escaping () -> Void
  ) {
    if response.actionIdentifier == UNNotificationDefaultActionIdentifier {
      keep(NotificationTaps.target(in: response.notification.request.content.userInfo))
    }
    if earlier?.userNotificationCenter?(center, didReceive: response, withCompletionHandler: completionHandler) == nil {
      completionHandler()
    }
  }
}

class PushPlugin: Plugin {
  @objc public func takeTarget(_ invoke: Invoke) {
    NotificationTaps.shared.watch()
    invoke.resolve(TargetAnswer(target: NotificationTaps.shared.take()))
  }

  @objc public func previews(_ invoke: Invoke) {
    guard let args = try? invoke.parseArgs(PreviewsArgs.self) else {
      invoke.reject("could not read the message previews")
      return
    }
    if SharedPreviews.save(Data(args.json.utf8)) {
      invoke.resolve()
    } else {
      invoke.reject("could not keep the message previews")
    }
  }

  @objc public func token(_ invoke: Invoke) {
    NotificationTaps.shared.watch()
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
  NotificationTaps.shared.watch()
  return PushPlugin()
}
