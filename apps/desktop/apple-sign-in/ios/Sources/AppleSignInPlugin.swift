import AuthenticationServices
import Tauri
import UIKit
import WebKit

class SignInRequest: Decodable {
  let nonce: String
}

class SheetRequest: Decodable {
  let url: String
}

class PasskeyRequest: Decodable {
  let request: String
}

struct AppleCredential: Encodable {
  let identityToken: String
  let givenName: String
  let familyName: String
}

struct SheetResult: Encodable {
  let url: String
}

struct PasskeyOptions: Decodable {
  let challenge: String
  let rpId: String?
  let allowCredentials: [AllowedCredential]?
  let userVerification: String?
}

struct AllowedCredential: Decodable {
  let id: String
}

struct PasskeyResponse: Encodable {
  let clientDataJSON: String
  let authenticatorData: String
  let signature: String
  let userHandle: String
}

struct PasskeyAssertion: Encodable {
  let id: String
  let rawId: String
  let type: String
  let authenticatorAttachment: String
  let response: PasskeyResponse
  let clientExtensionResults: [String: String]
}

extension Data {
  init?(base64URL: String) {
    var text = base64URL.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
    while text.count % 4 != 0 {
      text += "="
    }
    self.init(base64Encoded: text)
  }

  func base64URL() -> String {
    return base64EncodedString()
      .replacingOccurrences(of: "+", with: "-")
      .replacingOccurrences(of: "/", with: "_")
      .replacingOccurrences(of: "=", with: "")
  }
}

class AppleSignInPlugin: Plugin, ASAuthorizationControllerDelegate, ASAuthorizationControllerPresentationContextProviding,
  ASWebAuthenticationPresentationContextProviding
{
  private var pending: Invoke?
  private var controller: ASAuthorizationController?
  private var sheet: ASWebAuthenticationSession?
  private var sheetInvoke: Invoke?

  @objc public override func load(webview: WKWebView) {
    ScreenRegions.watch(webview)
  }

  @objc public func signIn(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(SignInRequest.self)
    DispatchQueue.main.async {
      let request = ASAuthorizationAppleIDProvider().createRequest()
      request.requestedScopes = [.fullName, .email]
      request.nonce = args.nonce
      self.perform(request, invoke)
    }
  }

  @objc public func passkey(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PasskeyRequest.self)
    guard #available(iOS 15.0, *) else {
      invoke.reject("Passkeys need iOS 15 or newer")
      return
    }
    guard
      let options = try? JSONDecoder().decode(PasskeyOptions.self, from: Data(args.request.utf8)),
      let challenge = Data(base64URL: options.challenge),
      let rpId = options.rpId, !rpId.isEmpty
    else {
      invoke.reject("The account server sent broken passkey options")
      return
    }
    DispatchQueue.main.async {
      let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: rpId)
      let request = provider.createCredentialAssertionRequest(challenge: challenge)
      request.allowedCredentials = (options.allowCredentials ?? [])
        .compactMap { Data(base64URL: $0.id) }
        .map { ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: $0) }
      request.userVerificationPreference = options.userVerification == "required" ? .required : .preferred
      self.perform(request, invoke)
    }
  }

  @objc public func openSheet(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(SheetRequest.self)
    guard let url = URL(string: args.url), url.scheme == "https" else {
      invoke.reject("That is not a sign-in page")
      return
    }
    DispatchQueue.main.async {
      if self.sheetInvoke != nil {
        invoke.reject("A sign-in page is already open")
        return
      }
      let session = ASWebAuthenticationSession(url: url, callbackURLScheme: "commy") { [weak self] callback, error in
        DispatchQueue.main.async {
          guard let self = self, let waiting = self.sheetInvoke else { return }
          self.sheetInvoke = nil
          self.sheet = nil
          if let callback = callback {
            waiting.resolve(SheetResult(url: callback.absoluteString))
          } else if let failure = error as? ASWebAuthenticationSessionError, failure.code == .canceledLogin {
            waiting.reject("canceled")
          } else {
            waiting.reject(error?.localizedDescription ?? "The sign-in page closed")
          }
        }
      }
      session.presentationContextProvider = self
      session.prefersEphemeralWebBrowserSession = false
      self.sheet = session
      self.sheetInvoke = invoke
      if !session.start() {
        self.sheet = nil
        self.sheetInvoke = nil
        invoke.reject("The sign-in page could not open")
      }
    }
  }

  private func perform(_ request: ASAuthorizationRequest, _ invoke: Invoke) {
    pending?.reject("Another sign-in started")
    pending = invoke
    let controller = ASAuthorizationController(authorizationRequests: [request])
    controller.delegate = self
    controller.presentationContextProvider = self
    self.controller = controller
    controller.performRequests()
  }

  private func keyWindow() -> ASPresentationAnchor {
    let windows = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap { $0.windows }
    return windows.first { $0.isKeyWindow } ?? windows.first ?? ASPresentationAnchor()
  }

  public func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
    return keyWindow()
  }

  public func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
    return keyWindow()
  }

  public func authorizationController(controller: ASAuthorizationController, didCompleteWithAuthorization authorization: ASAuthorization) {
    guard let invoke = finish() else { return }
    if #available(iOS 15.0, *), let assertion = authorization.credential as? ASAuthorizationPlatformPublicKeyCredentialAssertion {
      let id = assertion.credentialID.base64URL()
      let authenticatorData: Data? = assertion.rawAuthenticatorData
      let signature: Data? = assertion.signature
      let userID: Data? = assertion.userID
      invoke.resolve(
        PasskeyAssertion(
          id: id,
          rawId: id,
          type: "public-key",
          authenticatorAttachment: "platform",
          response: PasskeyResponse(
            clientDataJSON: assertion.rawClientDataJSON.base64URL(),
            authenticatorData: (authenticatorData ?? Data()).base64URL(),
            signature: (signature ?? Data()).base64URL(),
            userHandle: (userID ?? Data()).base64URL()
          ),
          clientExtensionResults: [:]
        )
      )
      return
    }
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
