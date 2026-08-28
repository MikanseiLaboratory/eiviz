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
