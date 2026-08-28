# eiviz

C# WPF host + Rust wgpu (DX12) mixer. OMT/VMX use the pure-Rust crates
[`openmediatransport-rs`](https://github.com/MikanseiLaboratory/openmediatransport-rs)
and [`vmx-rs`](https://github.com/MikanseiLaboratory/vmx-rs).

```powershell
dotnet build eiviz.slnx -c Release
dotnet run --project host\Eiviz.Host.csproj -c Release
cargo test --manifest-path mixer\Cargo.toml
```
