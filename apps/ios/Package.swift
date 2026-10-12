// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "Qmsg",
    platforms: [.iOS(.v18), .macOS(.v15)],
    products: [.library(name: "Qmsg", targets: ["Qmsg"])],
    dependencies: [.package(path: "QmsgKit")],
    targets: [
        .target(
            name: "Qmsg",
            dependencies: [.product(name: "QmsgKit", package: "QmsgKit")],
            path: "Qmsg",
            exclude: ["Info.plist", "Assets.xcassets"],
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [
                .unsafeFlags(["-Xlinker", "-platform_version", "-Xlinker", "ios",
                              "-Xlinker", "18.0", "-Xlinker", "27.1"],
                             .when(platforms: [.iOS])),
            ]
        ),
    ]
)
