# eiviz
This project is work-in-progress!  
eiviz / 映像(eizou) + visual

<img width="1916" height="1030" alt="image" src="https://github.com/user-attachments/assets/7b2f30c0-7870-49d7-9fdc-369da2e10ef4" />


Proof-of-Concept multi M/E vision mixer.  
Rust wgpu (DX12) mixer with C# WPF UI. 　

```powershell
dotnet build eiviz.slnx -c Release
dotnet run --project host\Eiviz.Host.csproj -c Release
cargo test --manifest-path mixer\Cargo.toml
```

## License

eiviz original source is licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE).

Non-commercial use is allowed: personal, hobby, education, research, and use by qualifying noncommercial organizations. Selling eiviz, bundling it as a product, or other commercial purposes are not allowed under this license. A separate commercial license from Shugo Kawamura / Mikansei Laboratory is required for those uses.

Third-party crates and libraries stay under their original MIT / Apache-2.0 / Zlib terms. See [NOTICE](NOTICE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
