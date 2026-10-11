import SwiftUI
import UIKit
import AVFoundation
import VisionKit
import QmsgKit

struct PairingView: View {
    @Environment(QmsgStore.self) private var store
    @State private var code = ""
    @State private var deviceName = UIDevice.current.name
    @State private var scanning = false
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    VStack(alignment: .leading, spacing: 12) {
                        Image(systemName: "bubble.left.and.bubble.right.fill")
                            .font(.largeTitle).foregroundStyle(.tint)
                        Text("Your chats, together.").font(.title2.bold())
                        Text("Scan the pairing QR code from your qmsg server, or paste its pairing data below.")
                            .foregroundStyle(.secondary)
                        if store.needsPairing {
                            Text("This device is no longer paired. Create a new pairing code on your server.")
                                .foregroundStyle(.orange)
                        }
                    }.padding(.vertical, 12)
                }
                Section("Pair with your server") {
                    if DataScannerViewController.isSupported {
                        Button("Scan QR code", systemImage: "qrcode.viewfinder") {
                            Task {
                                if await AVCaptureDevice.requestAccess(for: .video), DataScannerViewController.isAvailable {
                                    scanning = true
                                } else {
                                    error = "Allow camera access in Settings to scan, or paste the pairing data."
                                }
                            }
                        }
                    }
                    TextField("Device name", text: $deviceName)
                    TextField("Pairing data", text: $code, axis: .vertical)
                        .lineLimit(3...6)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                    if let error { Text(error).foregroundStyle(.red) }
                    Button {
                        pair()
                    } label: {
                        HStack {
                            Text("Connect")
                            Spacer()
                            if busy { ProgressView() }
                        }
                    }.disabled(busy || code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
            .disabled(busy)
            .navigationTitle("qmsg")
            .sheet(isPresented: $scanning) {
                NavigationStack {
                    QRScanner { result in
                        switch result {
                        case .success(let value): code = value
                        case .failure(let failure): error = failure.localizedDescription
                        }
                        scanning = false
                    }
                    .ignoresSafeArea(edges: .bottom)
                    .navigationTitle("Scan pairing code")
                    .toolbar { Button("Cancel") { scanning = false } }
                }
            }
        }
    }

    private func pair() {
        busy = true
        error = nil
        Task {
            defer { busy = false }
            do { try await store.pair(code: code, deviceName: deviceName.isEmpty ? "iPhone" : deviceName) }
            catch { self.error = QmsgStore.describe(error) }
        }
    }
}

@MainActor
private struct QRScanner: UIViewControllerRepresentable {
    var onResult: (Result<String, Error>) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(onResult: onResult) }

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let scanner = DataScannerViewController(
            recognizedDataTypes: [.barcode(symbologies: [.qr])],
            qualityLevel: .balanced,
            recognizesMultipleItems: false,
            isHighFrameRateTrackingEnabled: false,
            isHighlightingEnabled: true
        )
        scanner.delegate = context.coordinator
        do { try scanner.startScanning() }
        catch { DispatchQueue.main.async { context.coordinator.finish(.failure(error)) } }
        return scanner
    }

    func updateUIViewController(_ scanner: DataScannerViewController, context: Context) {}
    static func dismantleUIViewController(_ scanner: DataScannerViewController, coordinator: Coordinator) {
        scanner.stopScanning()
    }

    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        let onResult: (Result<String, Error>) -> Void
        private var finished = false
        init(onResult: @escaping (Result<String, Error>) -> Void) { self.onResult = onResult }

        func finish(_ result: Result<String, Error>) {
            guard !finished else { return }
            finished = true
            onResult(result)
        }

        func dataScanner(_ dataScanner: DataScannerViewController, didAdd addedItems: [RecognizedItem], allItems: [RecognizedItem]) {
            for case .barcode(let barcode) in addedItems {
                if let payload = barcode.payloadStringValue { finish(.success(payload)); return }
            }
        }

        func dataScanner(_ dataScanner: DataScannerViewController, becameUnavailableWithError error: DataScannerViewController.ScanningUnavailable) {
            finish(.failure(error))
        }
    }
}
