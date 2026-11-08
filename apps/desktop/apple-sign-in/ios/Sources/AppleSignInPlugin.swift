import AuthenticationServices
import Tauri
import UIKit

class SignInRequest: Decodable {
  let nonce: String
}

struct AppleCredential: Encodable {
  let identityToken: String
  let givenName: String
  let familyName: String
}

class AppleSignInPlugin: Plugin, ASAuthorizationControllerDelegate, ASAuthorizationControllerPresentationContextProviding {
  private var pending: Invoke?
  private var controller: ASAuthorizationController?

  @objc public func signIn(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(SignInRequest.self)
    DispatchQueue.main.async {
      self.pending?.reject("Another Apple sign-in started")
      self.pending = invoke
      let request = ASAuthorizationAppleIDProvider().createRequest()
      request.requestedScopes = [.fullName, .email]
      request.nonce = args.nonce
      let controller = ASAuthorizationController(authorizationRequests: [request])
      controller.delegate = self
      controller.presentationContextProvider = self
      self.controller = controller
      controller.performRequests()
    }
  }

  public func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
    let windows = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap { $0.windows }
    return windows.first { $0.isKeyWindow } ?? windows.first ?? ASPresentationAnchor()
  }

  public func authorizationController(controller: ASAuthorizationController, didCompleteWithAuthorization authorization: ASAuthorization) {
    guard let invoke = finish() else { return }
    guard
      let credential = authorization.credential as? ASAuthorizationAppleIDCredential,
      let data = credential.identityToken,
      let token = String(data: data, encoding: .utf8)
    else {
      invoke.reject("Apple did not send a sign-in token")
      return
    }
    invoke.resolve(
      AppleCredential(
        identityToken: token,
        givenName: credential.fullName?.givenName ?? "",
        familyName: credential.fullName?.familyName ?? ""
      )
    )
  }

  public func authorizationController(controller: ASAuthorizationController, didCompleteWithError error: Error) {
    guard let invoke = finish() else { return }
    if let failure = error as? ASAuthorizationError, failure.code == .canceled {
      invoke.reject("canceled")
    } else {
      invoke.reject(error.localizedDescription)
    }
  }

  private func finish() -> Invoke? {
    let invoke = pending
    pending = nil
    controller = nil
    return invoke
  }
}

@_cdecl("init_plugin_commy_apple_sign_in")
func initPlugin() -> Plugin {
  return AppleSignInPlugin()
}
