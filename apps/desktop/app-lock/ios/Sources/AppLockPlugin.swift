import LocalAuthentication
import Tauri
import UIKit
import WebKit

class UnlockRequest: Decodable {
  let reason: String
}

class CoverRequest: Decodable {
  let on: Bool
}

struct LockCheck: Encodable {
  let secure: Bool
  let biometry: String
}

class AppLockPlugin: Plugin {
  private var coverWanted = false
  private var cover: UIView?
  private var asking: LAContext?

  @objc public override func load(webview: WKWebView) {
    let center = NotificationCenter.default
    center.addObserver(self, selector: #selector(leaving), name: UIApplication.willResignActiveNotification, object: nil)
    center.addObserver(self, selector: #selector(leaving), name: UIApplication.didEnterBackgroundNotification, object: nil)
    center.addObserver(self, selector: #selector(returning), name: UIApplication.didBecomeActiveNotification, object: nil)
  }

  @objc public func check(_ invoke: Invoke) {
    let context = LAContext()
    let secure = context.canEvaluatePolicy(.deviceOwnerAuthentication, error: nil)
    _ = context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil)
    invoke.resolve(LockCheck(secure: secure, biometry: AppLockPlugin.biometryName(context.biometryType)))
  }

  @objc public func unlock(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(UnlockRequest.self)
    DispatchQueue.main.async {
      let context = LAContext()
      self.asking = context
      context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: args.reason) { passed, error in
        DispatchQueue.main.async {
          self.asking = nil
          if passed {
            invoke.resolve()
          } else {
            invoke.reject(AppLockPlugin.failure(error))
          }
        }
      }
    }
  }

  @objc public func cover(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(CoverRequest.self)
    DispatchQueue.main.async {
      self.coverWanted = args.on
      if !args.on {
        self.uncover()
      }
      invoke.resolve()
    }
  }

  @objc private func leaving() {
    guard coverWanted, asking == nil, cover == nil, let window = AppLockPlugin.frontWindow() else { return }
    let blur = UIVisualEffectView(effect: UIBlurEffect(style: .systemThickMaterial))
    blur.frame = window.bounds
    blur.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    window.addSubview(blur)
    cover = blur
  }

  @objc private func returning() {
    uncover()
  }

  private func uncover() {
    cover?.removeFromSuperview()
    cover = nil
  }

  static func frontWindow() -> UIWindow? {
    let windows = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap { $0.windows }
    return windows.first { $0.isKeyWindow } ?? windows.first
  }

  static func biometryName(_ kind: LABiometryType) -> String {
    switch kind {
    case .faceID:
      return "face"
    case .touchID:
      return "touch"
    default:
      if #available(iOS 17.0, *), kind == .opticID {
        return "optic"
      }
      return "none"
    }
  }

  static func failure(_ error: Error?) -> String {
    guard let error = error as? LAError else { return "Commy could not check that it is you" }
    switch error.code {
    case .userCancel, .appCancel, .systemCancel, .userFallback:
      return "canceled"
    case .passcodeNotSet:
      return "no-lock"
    case .biometryLockout:
      return "Too many tries. Unlock your phone with its passcode, then try again."
    default:
      return error.localizedDescription
    }
  }
}

@_cdecl("init_plugin_commy_app_lock")
func initPlugin() -> Plugin {
  return AppLockPlugin()
}
