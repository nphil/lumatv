#!/usr/bin/env node

const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

// Version and description come from appinfo.json; every name from brand.json.
const repoRoot = path.resolve(__dirname, '..');
const appinfo = JSON.parse(fs.readFileSync(path.join(repoRoot, 'appinfo.json'), 'utf8'));
const brand = JSON.parse(fs.readFileSync(path.join(repoRoot, 'brand.json'), 'utf8'));

const version = appinfo.version;
const id = brand.appId;

// Get IPK path from arguments
if (process.argv.length < 3) {
    console.error('Usage: gen-manifest.js <ipk-path> [output-file]');
    process.exit(1);
}

const ipkPath = process.argv[2];
const outputFile = process.argv[3] || null;

// Verify IPK exists
if (!fs.existsSync(ipkPath)) {
    console.error(`Error: IPK not found: ${ipkPath}`);
    process.exit(1);
}

// Compute SHA256 hash of IPK
const fileBuffer = fs.readFileSync(ipkPath);
const hashSum = crypto.createHash('sha256');
hashSum.update(fileBuffer);
const sha256 = hashSum.digest('hex');

// Construct manifest
const ipkFilename = path.basename(ipkPath);
const ipkUrl = `https://github.com/${brand.repo}/releases/download/v${version}/${ipkFilename}`;
const iconUri = `https://raw.githubusercontent.com/${brand.repo}/main/assets/icon160.png`;
const sourceUrl = `https://github.com/${brand.repo}`;

const manifest = {
    id,
    version,
    type: 'web',
    title: brand.displayName,
    appDescription: appinfo.appDescription,
    iconUri,
    sourceUrl,
    rootRequired: true,
    ipkUrl,
    ipkHash: {
        sha256
    }
};

// Output
const output = JSON.stringify(manifest, null, 2);

if (outputFile) {
    fs.writeFileSync(outputFile, output + '\n', 'utf8');
    console.log(`Generated: ${outputFile}`);
    console.log(`  Version: ${version}`);
    console.log(`  SHA256: ${sha256}`);
} else {
    console.log(output);
}
