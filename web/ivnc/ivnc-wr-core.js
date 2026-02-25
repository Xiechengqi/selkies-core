/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 *
 * This file incorporates work covered by the following copyright and
 * permission notice:
 *
 *   Copyright 2019 Google LLC
 *
 *   Licensed under the Apache License, Version 2.0 (the "License");
 *   you may not use this file except in compliance with the License.
 *   You may obtain a copy of the License at
 *
 *        http://www.apache.org/licenses/LICENSE-2.0
 *
 *   Unless required by applicable law or agreed to in writing, software
 *   distributed under the License is distributed on an "AS IS" BASIS,
 *   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 *   See the License for the specific language governing permissions and
 *   limitations under the License.
 */

import { WebRTCDemo } from "./lib/webrtc.js?v=24";
import { WebRTCDemoSignaling } from "./lib/signaling.js?v=1";
import { stringToBase64 } from "./lib/util.js?v=1";
import { Input } from "./lib/input2.js?v=18";

function InitUI() {
	let style = document.createElement('style');
	style.textContent = `
	body {
		background-color: #000000;
		font-family: sans-serif;
		margin: 0;
		padding: 0;
		overflow: hidden;
		background-color: #000;
		color: #fff;
	}

	#app {
		display: flex;
		flex-direction: column;
		height: calc(var(--vh, 1vh) * 100);
		width: 100%;
	}

	.video-container {
		flex-grow: 1;
		flex-shrink: 1;
		display: flex;
		flex-direction: column;
		justify-content: center;
		align-items: center;
		height: 100%;
		width: 100%;
		position: relative;
		overflow: hidden;
	}

	.video-container video,
	.video-container #overlayInput{
		position: absolute;
		top: 0;
		left: 0;
		width: 100%;
		height: 100%;
	}

	.video-container video {
		object-fit: fill;
	}

	.video-container #overlayInput {
		opacity: 0;
		z-index: 3;
		caret-color: transparent;
		background-color: transparent;
		color: transparent;
		pointer-events: auto;
		-webkit-user-select: none;
		border: none;
		outline: none;
		padding: 0;
		margin: 0;
	}

	.video-container #playButton {
		position: absolute;
		top: 50%;
		left: 50%;
		transform: translate(-50%, -50%);
		z-index: 10;
	}

	.video-container .status-bar {
		position: absolute;
		bottom: 0;
		left: 0;
		width: 100%;
		padding: 5px;
		background-color: rgba(0, 0, 0, 0.7);
		color: #fff;
		text-align: center;
		z-index: 5;
	}

	.loading-text {
		margin-top: 1em;
	}

	.hidden {
		display: none !important;
	}

	#playButton {
		padding: 15px 30px;
		font-size: 1.5em;
		cursor: pointer;
		background-color: rgba(0, 0, 0, 0.5);
		color: white;
		border: 1px solid rgba(255, 255, 255, 0.3);
		border-radius: 3px;
		backdrop-filter: blur(5px);
	}
	.no-window-overlay {
		position: absolute;
		top: 0;
		left: 0;
		width: 100%;
		height: 100%;
		background: #f5f5f5;
		display: flex;
		justify-content: center;
		align-items: center;
		z-index: 1000;
		color: #333;
		font-family: system-ui, sans-serif;
	}
	.no-window-overlay.hidden { display: none; }
	.no-window-content { text-align: center; }
	.no-window-content h2 { font-size: 24px; margin-bottom: 10px; }
	.no-window-content p { font-size: 14px; color: #666; }

	.taskbar {
		position: fixed;
		bottom: 0;
		left: 0;
		width: 100%;
		height: 36px;
		background: rgba(30, 30, 30, 0.85);
		backdrop-filter: blur(8px);
		display: flex;
		align-items: center;
		padding: 0 4px;
		z-index: 1000;
		box-sizing: border-box;
		transform: translateY(100%);
		transition: transform 0.15s ease;
		pointer-events: auto;
	}
	.taskbar.visible {
		transform: translateY(0);
	}
	.taskbar.pinned {
		transform: translateY(0);
	}
	.taskbar-pin {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 28px;
		height: 28px;
		margin: 0 4px 0 2px;
		border-radius: 4px;
		background: rgba(255, 255, 255, 0.08);
		color: #888;
		font-size: 14px;
		cursor: pointer;
		flex-shrink: 0;
		border: 1px solid transparent;
		transition: background 0.1s;
	}
	.taskbar-pin:hover {
		background: rgba(255, 255, 255, 0.15);
	}
	.taskbar-pin.active {
		color: #4c86e6;
		background: rgba(76, 134, 230, 0.2);
		border-color: rgba(76, 134, 230, 0.4);
	}
	.taskbar-conn {
		position: absolute;
		right: 8px;
		top: 0;
		padding: 0 4px;
		font-size: 11px;
		line-height: 36px;
		color: rgba(255, 255, 255, 0.7);
		white-space: nowrap;
		user-select: none;
		pointer-events: none;
	}
	.taskbar-trigger {
		position: fixed;
		bottom: 0;
		left: 0;
		width: 100%;
		height: 6px;
		z-index: 999;
	}
	.taskbar-item {
		display: inline-flex;
		align-items: center;
		height: 28px;
		padding: 0 12px;
		margin: 0 2px;
		border-radius: 4px;
		background: rgba(255, 255, 255, 0.08);
		color: #ccc;
		font-size: 12px;
		font-family: system-ui, sans-serif;
		cursor: pointer;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
		min-width: 120px;
		max-width: 200px;
		user-select: none;
		border: 1px solid transparent;
		transition: background 0.1s;
	}
	.taskbar-item:hover {
		background: rgba(255, 255, 255, 0.15);
	}
	.taskbar-item.focused {
		background: rgba(76, 134, 230, 0.35);
		color: #fff;
		border-color: rgba(76, 134, 230, 0.6);
	}
	.taskbar-close {
		display: none;
		margin-left: 6px;
		width: 14px;
		height: 14px;
		line-height: 12px;
		text-align: center;
		border-radius: 2px;
		background: rgba(255, 255, 255, 0.15);
		font-size: 10px;
		cursor: pointer;
		flex-shrink: 0;
	}
	.taskbar-close:hover {
		background: rgba(232, 89, 89, 0.8);
	}
	.taskbar-item.focused .taskbar-close {
		display: inline-block;
	}
	.pwd-overlay {
		position: fixed;
		inset: 0;
		background: rgba(0,0,0,0.6);
		z-index: 2000;
		display: flex;
		align-items: center;
		justify-content: center;
	}
	.pwd-dialog {
		background: #1e1e1e;
		border: 1px solid #444;
		border-radius: 8px;
		padding: 24px;
		min-width: 300px;
		color: #eee;
		font-family: system-ui, sans-serif;
	}
	.pwd-dialog h3 {
		margin: 0 0 16px;
		font-size: 15px;
		font-weight: 600;
	}
	.pwd-dialog input {
		display: block;
		width: 100%;
		padding: 8px;
		margin-bottom: 10px;
		border: 1px solid #555;
		border-radius: 4px;
		background: #2a2a2a;
		color: #eee;
		font-size: 13px;
		box-sizing: border-box;
	}
	.pwd-dialog input:focus {
		outline: none;
		border-color: #4c86e6;
	}
	.pwd-dialog .pwd-btns {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
		margin-top: 14px;
	}
	.pwd-dialog button {
		padding: 6px 16px;
		border: none;
		border-radius: 4px;
		font-size: 13px;
		cursor: pointer;
	}
	.pwd-dialog .pwd-cancel {
		background: #444;
		color: #ccc;
	}
	.pwd-dialog .pwd-cancel:hover {
		background: #555;
	}
	.pwd-dialog .pwd-ok {
		background: #4c86e6;
		color: #fff;
	}
	.pwd-dialog .pwd-ok:hover {
		background: #5a94f0;
	}
	.pwd-msg {
		font-size: 12px;
		margin-top: 8px;
		min-height: 16px;
	}
	.pwd-msg.error { color: #e85959; }
	.pwd-msg.ok { color: #5cb85c; }
	`;
  document.head.appendChild(style);
}

function showChangePasswordModal() {
	// Remove existing modal if any
	const existing = document.querySelector('.pwd-overlay');
	if (existing) existing.remove();

	const overlay = document.createElement('div');
	overlay.className = 'pwd-overlay';

	const dialog = document.createElement('div');
	dialog.className = 'pwd-dialog';
	dialog.innerHTML = `
		<h3>修改密码</h3>
		<input type="password" id="pwd-new" placeholder="新密码 (至少4位)" autocomplete="new-password" />
		<input type="password" id="pwd-confirm" placeholder="确认新密码" autocomplete="new-password" />
		<div class="pwd-msg" id="pwd-msg"></div>
		<div class="pwd-btns">
			<button class="pwd-cancel" id="pwd-cancel">取消</button>
			<button class="pwd-ok" id="pwd-ok">确定</button>
		</div>
	`;
	overlay.appendChild(dialog);
	document.body.appendChild(overlay);

	const newInput = document.getElementById('pwd-new');
	const confirmInput = document.getElementById('pwd-confirm');
	const msg = document.getElementById('pwd-msg');
	const okBtn = document.getElementById('pwd-ok');
	const cancelBtn = document.getElementById('pwd-cancel');

	newInput.focus();

	const close = () => overlay.remove();
	overlay.addEventListener('click', (e) => { if (e.target === overlay) close(); });
	cancelBtn.addEventListener('click', close);

	okBtn.addEventListener('click', async () => {
		const np = newInput.value;
		const cp = confirmInput.value;
		msg.className = 'pwd-msg';
		msg.textContent = '';

		if (np.length < 4) {
			msg.className = 'pwd-msg error';
			msg.textContent = '密码至少需要4个字符';
			return;
		}
		if (np !== cp) {
			msg.className = 'pwd-msg error';
			msg.textContent = '两次输入的密码不一致';
			return;
		}

		okBtn.disabled = true;
		okBtn.textContent = '...';
		try {
			const resp = await fetch('/api/change-password', {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ new_password: np }),
			});
			if (resp.ok) {
				msg.className = 'pwd-msg ok';
				msg.textContent = '密码已修改，下次请求将使用新密码';
				setTimeout(close, 1500);
			} else {
				const data = await resp.json().catch(() => ({}));
				msg.className = 'pwd-msg error';
				msg.textContent = data.error || '修改失败';
			}
		} catch (e) {
			msg.className = 'pwd-msg error';
			msg.textContent = '网络错误';
		}
		okBtn.disabled = false;
		okBtn.textContent = '确定';
	});
}

export default function webrtc() {
	let appName;
	let videoBitRate = 8000;
	let videoFramerate = 60;
	let audioBitRate = 96000;
	let showStart = false;
	let showDrawer = false;
	// TODO: how do we want to handle the log and debug entries
	let logEntries = [];
	let debugEntries = [];
	let status = 'connecting';
	let clipboardStatus = 'enabled';
	let windowResolution = "";
	let encoderLabel = "";
	let encoder = ""

	let connectionStat = {
		connectionStatType: "unknown",
		connectionLatency: 0,
		connectionVideoLatency: 0,
		connectionAudioLatency: 0,
		connectionAudioCodecName: "NA",
		connectionAudioBitrate: 0,
		connectionPacketsReceived: 0,
		connectionPacketsLost: 0,
		connectionBytesReceived: 0,
		connectionBytesSent: 0,
		connectionCodec: "unknown",
		connectionVideoDecoder: "unknown",
		connectionResolution: "",
		connectionFrameRate: 0,
		connectionVideoBitrate: 0,
		connectionAvailableBandwidth: 0
	};

	var videoElement = null;
	var audioElement = null;
	let serverLatency = 0;
	let resizeRemote = false;
	let scaleLocal = false;
	let debug = false;
	let playButtonElement = null;
	let statusDisplayElement = null;
	let rtime = null;
	let rdelta = 500; // time in milliseconds
	let rtimeout = false;
	let manualWidth = 0, manualHeight = 0;
	window.isManualResolutionMode = false;
	window.fps = 0;

	var videoConnected = "";
	var audioConnected = "";
	var statWatchEnabled = false;
	var statsLoopId = null;
	var metricsLoopId = null;
	var webrtc = null;
	var input = null;
	let useCssScaling = true;

	const UPLOAD_CHUNK_SIZE = 64 * 1024  - 1; // 64KiB, excluding a byte for prefix

	// Set storage key based on URL
	const urlForKey = window.location.href.split('#')[0];
	const storageAppName = urlForKey.replace(/[^a-zA-Z0-9.-_]/g, '_');
	const _urlParams = new URLSearchParams(window.location.search);

	const getIntParam = (key, default_value) => {
		const prefixedKey = `${storageAppName}_${key}`;
		const value = window.localStorage.getItem(prefixedKey);
		return (value === null || value === undefined) ? default_value : parseInt(value);
	};
	const setIntParam = (key, value) => {
		const prefixedKey = `${storageAppName}_${key}`;
		if (value === null || value === undefined) {
				window.localStorage.removeItem(prefixedKey);
		} else {
				window.localStorage.setItem(prefixedKey, value.toString());
		}
	};
	const getBoolParam = (key, default_value) => {
		if (_urlParams.has(key)) {
			return _urlParams.get(key).toLowerCase() === 'true';
		}
		const prefixedKey = `${storageAppName}_${key}`;
		const v = window.localStorage.getItem(prefixedKey);
		if (v === null) {
				return default_value;
		}
		return v.toString().toLowerCase() === 'true';
	};
	const setBoolParam = (key, value) => {
		const prefixedKey = `${storageAppName}_${key}`;
		if (value === null || value === undefined) {
				window.localStorage.removeItem(prefixedKey);
		} else {
				window.localStorage.setItem(prefixedKey, value.toString());
		}
	};
	const getStringParam = (key, default_value) => {
		const prefixedKey = `${storageAppName}_${key}`;
		const value = window.localStorage.getItem(prefixedKey);
		return (value === null || value === undefined) ? default_value : value;
	};
	const setStringParam = (key, value) => {
		const prefixedKey = `${storageAppName}_${key}`;
		if (value === null || value === undefined) {
				window.localStorage.removeItem(prefixedKey);
		} else {
				window.localStorage.setItem(prefixedKey, value.toString());
		}
	};

	// Function to add timestamp to logs.
	var applyTimestamp = (msg) => {
		var now = new Date();
		var ts = now.getHours() + ":" + now.getMinutes() + ":" + now.getSeconds();
		return "[" + ts + "]" + " " + msg;
	}

	const roundDownToEven = (num) => {
		return Math.floor(num / 2) * 2;
	};

	function playStream() {
		showStart = false;
		if (playButtonElement) playButtonElement.classList.add('hidden');
		webrtc.playStream();
		webrtc.unmuteAudio();
	}

	function updateStatusDisplay() {
		if (statusDisplayElement) {
			statusDisplayElement.textContent = status;
			if (status == 'connected') {
				// clear the status and show the play button
				statusDisplayElement.classList.add("hidden");
				if (playButtonElement && showStart) {
					playButtonElement.classList.remove('hidden');
				}
			}
		}
	}

	function updateVideoImageRendering(){
		if (!videoElement) return;

		const dpr = window.devicePixelRatio || 1;
		const isOneToOne = !useCssScaling || (useCssScaling && dpr <= 1);
		if (isOneToOne) {
			// Use 'pixelated' for a sharp, 1:1 pixel look
			if (videoElement.style.imageRendering !== 'pixelated') {
				console.log("Setting video rendering to 'pixelated' for sharp display.");
				videoElement.style.imageRendering = 'pixelated';
			}
		} else {
			// Use 'auto' to let the browser smooth the upscaled video
			if (videoElement.style.imageRendering !== 'auto') {
				console.log("Setting video rendering to 'auto' for smooth upscaling.");
				videoElement.style.imageRendering = 'auto';
			}
		}
	};

	function sanitizeAndStoreSettings(serverSettings) {
		console.log("Sanitizing and storing settings based on server payload.");
		const changes = {};

		for (const key in serverSettings) {
			if (!serverSettings.hasOwnProperty(key)) continue;
			const setting = serverSettings[key];
			let sanitizedValue;
			if (setting.min !== undefined && setting.max !== undefined) {
				const clientValue = getIntParam(key, setting.default);
				if (clientValue < setting.min || clientValue > setting.max) {
					sanitizedValue = setting.default;
					console.log(`Sanitizing '${key}': value ${clientValue} is out of range [${setting.min}-${setting.max}]. Resetting to default ${sanitizedValue}.`);
					changes[key] = sanitizedValue;
				} else {
					sanitizedValue = clientValue;
				}
				window[key] = sanitizedValue;
				setIntParam(key, sanitizedValue);
			}
			else if (setting.allowed !== undefined) {
				const isNumericEnum = !isNaN(parseFloat(setting.allowed[0]));
				let clientValueStr;

				if (isNumericEnum) {
					clientValueStr = getIntParam(key, parseInt(setting.value, 10)).toString();
				} else {
					clientValueStr = getStringParam(key, setting.value);
				}

				if (!setting.allowed.includes(clientValueStr)) {
					sanitizedValue = setting.value;
					console.log(`Sanitizing '${key}': value "${clientValueStr}" is not in allowed list [${setting.allowed.join(', ')}]. Resetting to default "${sanitizedValue}".`);
					changes[key] = sanitizedValue;
				} else {
					sanitizedValue = clientValueStr;
				}

				if (isNumericEnum) {
					const numericValue = parseInt(sanitizedValue, 10);
					window[key] = numericValue;
					setIntParam(key, numericValue);
				} else {
					window[key] = sanitizedValue;
					setStringParam(key, sanitizedValue);
				}
			}
			else if (typeof setting.value === 'boolean') {
				const serverValue = setting.value;
				const isLocked = !!setting.locked;
				if (isLocked) {
					const clientValue = getBoolParam(key, !serverValue);
				if (clientValue !== serverValue) {
					console.log(`Sanitizing '${key}': setting is locked by server. Client value ${clientValue} is being overwritten with ${serverValue}.`);
					changes[key] = serverValue;
				}
				window[key] = serverValue;
				setBoolParam(key, serverValue);
				} else {
					const prefixedKey = `${storageAppName}_${key}`;
					const wasUnset = window.localStorage.getItem(prefixedKey) === null;
					const clientValue = getBoolParam(key, serverValue);
					if (wasUnset) {
						console.log(`Initializing unlocked setting '${key}' for the first time with server default: ${serverValue}. Flagging as a change.`);
						changes[key] = serverValue;
					}
					window[key] = clientValue;
					setBoolParam(key, clientValue);
				}
			}
		}
		return changes;
	}

	function sendClientPersistedSettings() {
		const settingsPrefix = `${storageAppName}_`;
		const settingsToSend = {};
		const dpr = useCssScaling ? 1 : (window.devicePixelRatio || 1);

		const knownSettings = [
			'framerate', 'encoder_rtc', 'is_manual_resolution_mode',
			'audio_bitrate', 'video_bitrate', 'scaling_dpi', 'enable_binary_clipboard'
		];
		const booleanSettingKeys = [
			'is_manual_resolution_mode', 'enable_binary_clipboard'
		];
		const integerSettingKeys = [
			'framerate', 'audio_bitrate', 'scaling_dpi', 'video_bitrate'
		];

		for (const key in localStorage) {
			if (Object.hasOwnProperty.call(localStorage, key) && key.startsWith(settingsPrefix)) {
				const unprefixedKey = key.substring(settingsPrefix.length);;
				const baseKey = unprefixedKey;
				if (knownSettings.includes(baseKey)) {
					let value = localStorage.getItem(key);
					if (booleanSettingKeys.includes(baseKey)) {
						value = (value === 'true');
					} else if (integerSettingKeys.includes(baseKey)) {
						value = parseInt(value, 10);
						if (isNaN(value)) continue;
					}
					settingsToSend[baseKey] = value;
				}
			}
		}

		if (window.isManualResolutionMode && manualWidth != null && manualHeight != null) {
			settingsToSend['is_manual_resolution_mode'] = true;
			settingsToSend['manual_width'] = roundDownToEven(manualWidth * dpr);
			settingsToSend['manual_height'] = roundDownToEven(manualHeight * dpr);
		}
		settingsToSend['useCssScaling'] = useCssScaling;

		try {
			const settingsJson = JSON.stringify(settingsToSend);
			webrtc.sendDataChannelMessage(`SETTINGS,${settingsJson}`);
		
			console.log('Sent initial settings to server:', settingsToSend);
		} catch (e) {
			console.error('Error constructing or sending initial settings:', e);
		}
	}

	function applyManualStyle(targetWidth, targetHeight, scaleToFit) {
		if (targetWidth <=0 || targetHeight <=0) {
			console.log("Invalid target height or width")
			return;
		}

		const dpr = (window.isManualResolutionMode || useCssScaling) ? 1 : (window.devicePixelRatio || 1);
		const logicalWidth = roundDownToEven(targetWidth * dpr);
		const logicalHeight = roundDownToEven(targetHeight * dpr);
		console.log(`applyManualStyle logicalWidth: ${logicalWidth} logicalHeight: ${logicalHeight}`)
		if (videoElement.width !== logicalWidth || videoElement.height !== logicalHeight) {
			videoElement.width = logicalWidth;
			videoElement.height = logicalHeight;
			console.log(`Video Element set to: ${targetWidth}x${targetHeight}`);
		}
		const container = videoElement.parentElement;
		const containerWidth = container.clientWidth;
		const containerHeight = container.clientHeight;
		if (scaleToFit) {
			const targetAspectRatio = targetWidth / targetHeight;
			const containerAspectRatio = containerWidth / containerHeight;
			let cssWidth, cssHeight;
			if (targetAspectRatio > containerAspectRatio) {
				cssWidth = containerWidth;
				cssHeight = containerWidth / targetAspectRatio;
			} else {
				cssHeight = containerHeight;
				cssWidth = containerHeight * targetAspectRatio;
			}
			const topOffset = (containerHeight - cssHeight) / 2;
			const leftOffset = (containerWidth - cssWidth) / 2;
			videoElement.style.position = 'absolute';
			videoElement.style.width = `${cssWidth}px`;
			videoElement.style.height = `${cssHeight}px`;
			videoElement.style.top = `${topOffset}px`;
			videoElement.style.left = `${leftOffset}px`;
			videoElement.style.objectFit = 'contain'; // Should be 'fill' if CSS handles aspect ratio
			console.log(`Applied manual style (Scaled): CSS ${cssWidth}x${cssHeight}, Pos ${leftOffset},${topOffset}`);
		} else {
			videoElement.style.position = 'absolute';
			videoElement.style.width = `${targetWidth}px`;
			videoElement.style.height = `${targetHeight}px`;
			videoElement.style.top = '0px';
			videoElement.style.left = '0px';
			videoElement.style.objectFit = 'contain';
			console.log(`Applied manual style (Exact): CSS ${targetWidth}x${targetHeight}, Pos 0,0`);
		}
		updateVideoImageRendering();
	}

	function resetToWindowResolution(targetWidth, targetHeight) {
		if (!videoElement) return;

		const dpr = useCssScaling ? 1 : (window.devicePixelRatio || 1);
		const logicalWidth = roundDownToEven(targetWidth * dpr);
		const logicalHeight = roundDownToEven(targetHeight * dpr);
		console.log(`resetToWinRes logicalWidth: ${logicalWidth} logicalHeight: ${logicalHeight}`)
		if (videoElement.width !== logicalWidth || videoElement.height !== logicalHeight) {
			videoElement.width = logicalWidth;
			videoElement.height = logicalHeight;
			console.log(`Video Element set to: ${logicalWidth}x${logicalHeight}`);
		}

		videoElement.style.position = 'absolute';
		videoElement.style.width = '100%';
		videoElement.style.height = '100%';
		videoElement.style.top = '0px';
		videoElement.style.left = '0px';
		videoElement.style.objectFit = 'fill';
		console.log(`Resized to window resolution: ${logicalWidth}x${logicalHeight}`);
	}

	function sendResolutionToServer(width, height) {
		const dpr = useCssScaling ? 1 : (window.devicePixelRatio || 1);
		const realWidth = roundDownToEven(width * dpr);
		const realHeight = roundDownToEven(height * dpr);
		const resString = `${realWidth}x${realHeight}`;
		console.log(`Sending resolution to server: ${resString}, Pixel Ratio Used: ${dpr}, useCssScaling: ${useCssScaling}`);
		webrtc.sendDataChannelMessage(`r,${resString}`);
	}

	function enableAutoResize() {
		window.addEventListener("resize", resizeStart);
	}

	function disableAutoResize() {
		window.removeEventListener("resize", resizeStart);
	}

	function resizeStart() {
		rtime = new Date();
		if (rtimeout === false) {
			rtimeout = true;
			setTimeout(() => { resizeEnd() }, rdelta);
		}
	}

	function resizeEnd() {
		if (new Date() - rtime < rdelta) {
			setTimeout(() => { resizeEnd() }, rdelta);
		} else {
			rtimeout = false;
			windowResolution = input.getWindowResolution();
			sendResolutionToServer(windowResolution[0], windowResolution[1])
			resetToWindowResolution(windowResolution[0], windowResolution[1])
		}
	}

	function loadLastSessionSettings() {
		// Preset the video element to last session resolution
		if (window.isManualResolutionMode && manualWidth && manualHeight) {
			console.log(`Applying manual resolution: ${manualWidth}x${manualHeight}`);
			applyManualStyle(manualWidth, manualHeight, scaleLocal);
		} else {
			console.log("Applying window resolution");
			// If manual resolution is not set, reset to window resolution
			const currentWindowRes = input.getWindowResolution();
			resetToWindowResolution(...currentWindowRes);
			sendResolutionToServer(currentWindowRes[0], currentWindowRes[1]);
			enableAutoResize();
		}
	}

	// callback invoked when "message" event is triggerd
	function handleMessage(event) {
		let message = event.data;
		switch(message.type) {
			case "setScaleLocally":
				if (typeof message.value === 'boolean') {
					console.log("Scaling the stream locally: ", message.value);
					// setScaleLocally returns true or false; false, to turn off the scaling
					if (message.value === true) disableAutoResize();
					scaleLocal = message.value;
					if (manualWidth && manualHeight) {
						applyManualStyle(manualWidth, manualHeight, scaleLocal);
						setBoolParam("scaleLocallyManual", scaleLocal);
					}
				} else {
					console.warn("Invalid value received for setScaleLocally:", message.value);
				}
				break;
			case "resetResolutionToWindow":
				console.log("Resetting to window size");
				manualHeight = manualWidth = 0; // clear manual W&H
				let currentWindowRes = input.getWindowResolution();
				resetToWindowResolution(...currentWindowRes);
				sendResolutionToServer(...currentWindowRes);
				enableAutoResize();
				setIntParam('manualWidth', null);
				setIntParam('manualHeight', null);
				setBoolParam('isManualResolutionMode', false);
				window.isManualResolutionMode = false;
				break;
			case "setManualResolution":
				const width = parseInt(message.width, 10);
				const height = parseInt(message.height, 10);
				if (isNaN(width) || width <= 0 || isNaN(height) || height <= 0) {
					console.error('Received invalid width/height for setManualResolution:', message);
					break;
				}
				console.log(`Setting manual resolution: ${width}x${height}`);
				disableAutoResize();
				manualWidth = width;
				manualHeight = height;
				applyManualStyle(manualWidth, manualHeight, scaleLocal);
				sendResolutionToServer(manualWidth, manualHeight);
				setIntParam('manualWidth', manualWidth);
				setIntParam('manualHeight', manualHeight);
				setBoolParam('isManualResolutionMode', true);
				window.isManualResolutionMode = true;
				break;
			case "setUseCssScaling":
				// TODO: fix issues with hiDPI especially from andriod clients
				// useCssScaling = message.value;
				// setBoolParam('useCssScaling', useCssScaling);
				// console.log(`Set useCssScaling to ${useCssScaling} and persisted.`);

				// input.updateCssScaling();
				// updateVideoImageRendering();
				// if (window.isManualResolutionMode && manualWidth != null && manualHeight != null) {
				//     sendResolutionToServer(manualWidth, manualHeight);
				//     applyManualStyle(manualWidth, manualHeight, scaleLocal);
				// } else {
				//     const currentWindowRes = input.getWindowResolution()
				//     const autoWidth = roundDownToEven(currentWindowRes[0]);
				//     const autoHeight = roundDownToEven(currentWindowRes[1]);
				//     sendResolutionToServer(autoWidth, autoHeight);
				//     resetToWindowResolution(autoWidth, autoHeight)
				// }
				console.warn("Skipping cssScaling since hidpi needs to be implemented")
				break;
			case "clipboardUpdateFromUI":
				console.log("Received clipboard from UI, sending it to server");
				webrtc.sendDataChannelMessage(`cw,${stringToBase64(message.text)}`);
				break;
			case "settings":
				console.log("Received settings msg from dashboard:", message.settings);
				handleSettingsMessage(message.settings);
				break;
			case "command":
				if (message.value !== null && message.value !== undefined) {
					const commandString = message.value;
					console.log(`Received 'command' message with value: "${commandString}"`);
					webrtc.sendDataChannelMessage(`cmd,${commandString}`);
				} else {
					console.warn(`Received invalid command from dashboard: ${message.value}`)
				}
				break;
		}
	}

	function handleSettingsMessage(settings) {
		if (settings.video_bitrate !== undefined) {
			videoBitRate = parseInt(settings.video_bitrate);
			webrtc.sendDataChannelMessage(`vb,${videoBitRate}`);
			setIntParam('video_bitrate', videoBitRate);
		}
		if (settings.framerate !== undefined) {
			videoFramerate = parseInt(settings.framerate);
			webrtc.sendDataChannelMessage(`_arg_fps,${videoFramerate}`);
			setIntParam('framerate', videoFramerate);
		}
		if (settings.audio_bitrate !== undefined) {
			audioBitRate = parseInt(settings.audio_bitrate);
			webrtc.sendDataChannelMessage(`ab,${audioBitRate}`);
			setIntParam('audio_bitrate', audioBitRate);
		}
		if (settings.encoder !== undefined) {
			console.log("Received encoder setting from dashboard:", settings.encoder);
			encoder = settings.encoder;
			console.warn("Changing of encoder on the fly is not yet supported");
			// setIntParam('encoder_rtc', encoder);
		}
		if (settings.SCALING_DPI !== undefined) {
			const dpi = parseInt(settings.SCALING_DPI, 10);
			webrtc.sendDataChannelMessage(`s,${dpi}`)
		}
	}

	function handleRequestFileUpload() {
		const hiddenInput = document.getElementById('globalFileInput');
		if (!hiddenInput) {
			console.error("Global file input not found!");
			return;
		}
		console.log("Triggering click on hidden file input.");
		hiddenInput.click();
	}

	async function handleFileInputChange(event) {
		const files = event.target.files;
		if (!files || files.length === 0) {
			event.target.value = null;
			return;
		}
		// For every user action 'upload' an auxiliary data is dynamically created.
		// Currently only one aux channel is allowed to operate at a given time, since the backend
		// doesn't support simultaneous reception of multiple files, yet.
		if (!webrtc.createAuxDataChannel()) {
			console.warn("Simultaneous uploading of files with distinct upload operations is not supported yet");
			const errorMsg = "Please let the ongoing upload complete";
			window.postMessage({
				type: 'fileUpload',
				payload: {
				status: 'warning',
				fileName: '_N/A_',
				message: errorMsg
				}
			}, window.location.origin);
			event.target.value = null;
			return;
		}
		console.log(`File input changed, processing ${files.length} files sequentially.`);
		try {
			await webrtc.waitForAuxChannelOpen();
			for (let i = 0; i < files.length; i++) {
				const file = files[i];
				const pathToSend = file.name;
				console.log(`Uploading file ${i + 1}/${files.length}: ${pathToSend}`);
				await uploadFileObject(file, pathToSend);
			}
			console.log("Finished processing all files from input.");
		} catch (error) {
			const errorMsg = `An error occurred during the file input upload process: ${error.message || error}`;
			console.error(errorMsg);
			window.postMessage({
				type: 'fileUpload',
				payload: {
				status: 'error',
				fileName: 'N/A',
				message: errorMsg
				}
			}, window.location.origin);
		} finally {
			event.target.value = null;
			webrtc.closeAuxDataChannel();
		}
	}

	function uploadFileObject(file, pathToSend) {
		return new Promise((resolve, reject) => {
			window.postMessage({
				type: 'fileUpload',
				payload: {
				status: 'start',
				fileName: pathToSend,
				fileSize: file.size
				}
			}, window.location.origin);
			webrtc.sendDataChannelMessage(`FILE_UPLOAD_START:${pathToSend}:${file.size}`)

			let offset = 0;
			const reader = new FileReader();
			reader.onload = async function(e) {
				if (e.target.error) {
					const readErrorMsg = `File read error for ${pathToSend}: ${e.target.error}`;
					window.postMessage({ type: 'fileUpload', payload: { status: 'error', fileName: pathToSend, message: readErrorMsg }}, window.location.origin);
					webrtc.sendDataChannelMessage(`FILE_UPLOAD_ERROR:${pathToSend}:File read error`)
					reject(e.target.error);
					return;
				}
				try {
					const prefixedView = new Uint8Array(1 + e.target.result.byteLength);
					prefixedView[0] = 0x01; // Data prefix for file chunk
					prefixedView.set(new Uint8Array(e.target.result), 1);
					webrtc.sendAuxChannelData(prefixedView.buffer);  // Using auxiliary data channel to send file data
					offset += e.target.result.byteLength;
					const progress = file.size > 0 ? Math.round((offset / file.size) * 100) : 100;
					window.postMessage({
						type: 'fileUpload',
						payload: {
						status: 'progress',
						fileName: pathToSend,
						progress: progress,
						fileSize: file.size
							}
					}, window.location.origin);
					if (offset < file.size) {
						if(webrtc.isAuxBufferNearThreshold()) {
							setTimeout(() => readChunk(offset), 50);
						} else {
							readChunk(offset)
						}
					} else {
						// Data channels work asynchronously due to their underlying implementation,
						// so we need to wait for its buffer to drain before sending the end message.
						await webrtc.awaitForAuxBufferToDrain();
						webrtc.sendDataChannelMessage(`FILE_UPLOAD_END:${pathToSend}`);
						window.postMessage({
						type: 'fileUpload',
						payload: {
							status: 'end',
							fileName: pathToSend,
							fileSize: file.size
						}
						}, window.location.origin);
						resolve();
						}
				} catch (error) {
					const sendErrorMsg = `error during upload of ${pathToSend}: ${error.message || error}`;
					window.postMessage({ type: 'fileUpload', payload: { status: 'error', fileName: pathToSend, message: sendErrorMsg }}, window.location.origin);
					webrtc.sendDataChannelMessage(`FILE_UPLOAD_ERROR:${pathToSend}:send error`);
					reject(error);
				}
			};
			reader.onerror = function(e) {
				const generalReadError = `General file reader error for ${pathToSend}: ${e.target.error}`;
				window.postMessage({ type: 'fileUpload', payload: { status: 'error', fileName: pathToSend, message: generalReadError }}, window.location.origin);
				webrtc.sendDataChannelMessage(`FILE_UPLOAD_ERROR:${pathToSend}:General file reader error`)
				reject(e.target.error);
			};

			function readChunk(startOffset) {
				const slice = file.slice(startOffset, Math.min(startOffset + UPLOAD_CHUNK_SIZE, file.size));
				reader.readAsArrayBuffer(slice);
			}
			readChunk(0);
		});
	}

	function handleDragOver(ev) {
		ev.preventDefault();
		ev.dataTransfer.dropEffect = 'copy';
	}

	async function handleDrop(ev) {
		ev.preventDefault();
		ev.stopPropagation();
		const entriesToProcess = [];
		if (!webrtc.createAuxDataChannel()) {
			console.warn("Simultaneous uploading of files with distinct upload operations is not supported yet");
			const errorMsg = "Please let the ongoing upload complete";
			window.postMessage({
				type: 'fileUpload',
				payload: {
				status: 'warning',
				fileName: '_N/A_',
				message: errorMsg
				}
			}, window.location.origin);
			return;
		}
		if (ev.dataTransfer.items) {
			for (let i = 0; i < ev.dataTransfer.items.length; i++) {
				const item = ev.dataTransfer.items[i];
			  // Only care about file-kind items
				if (item.kind !== 'file') continue;
				let entry = null;
				if (typeof item.webkitGetAsEntry === 'function') entry = item.webkitGetAsEntry();
				else if (typeof item.getAsEntry === 'function') entry = item.getAsEntry();
				if (entry) entriesToProcess.push(entry);
			}
		} else if (ev.dataTransfer.files.length > 0) {
			for (let i = 0; i < ev.dataTransfer.files.length; i++) {
				await uploadFileObject(ev.dataTransfer.files[i], ev.dataTransfer.files[i].name);
			}
			webrtc.closeAuxDataChannel();
			return;
		}

		// Process the nested entries
		try {
			for (const entry of entriesToProcess) await handleDroppedEntry(entry);
		} catch (error) {
			const errorMsg = `Error during sequential upload: ${error.message || error}`;
			window.postMessage({
				type: 'fileUpload',
				payload: {
				status: 'error',
				fileName: 'N/A',
				message: errorMsg
				}
			}, window.location.origin);
			webrtc.sendDataChannelMessage(`FILE_UPLOAD_ERROR:GENERAL:Processing failed`)
		}
		webrtc.closeAuxDataChannel();
	}

	function getFileFromEntry(fileEntry) {
		return new Promise((resolve, reject) => fileEntry.file(resolve, reject));
	}

	async function handleDroppedEntry(entry, basePathFallback = "") { // basePathFallback is for non-fullPath scenarios
		let pathToSend;
		if (entry.fullPath && typeof entry.fullPath === 'string' && entry.fullPath !== entry.name && (entry.fullPath.includes('/') || entry.fullPath.includes('\\'))) {
			pathToSend = entry.fullPath;
			if (pathToSend.startsWith('/')) {
				pathToSend = pathToSend.substring(1);
			}
			console.log(`Using entry.fullPath: "${pathToSend}" for entry.name: "${entry.name}"`);
		} else {
			pathToSend = basePathFallback ? `${basePathFallback}/${entry.name}` : entry.name;
			console.log(`Constructed path: "${pathToSend}" for entry.name: "${entry.name}" (basePathFallback: "${basePathFallback}")`);
		}

		if (entry.isFile) {
			try {
				const file = await getFileFromEntry(entry);
				await uploadFileObject(file, pathToSend);
			} catch (err) {
				console.error(`Error processing file ${pathToSend}: ${err}`);
				window.postMessage({
				type: 'fileUpload',
				payload: { status: 'error', fileName: pathToSend, message: `Error processing file: ${err.message || err}` }
				}, window.location.origin);
				webrtc.sendDataChannelMessage(`FILE_UPLOAD_ERROR:${pathToSend}:Client-side file processing error`)
			}
		} else if (entry.isDirectory) {
			console.log(`Processing directory: ${pathToSend}`);
			const dirReader = entry.createReader();
			let entries;
			do {
				entries = await new Promise((resolve, reject) => dirReader.readEntries(resolve, reject));
				for (const subEntry of entries) {
					await handleDroppedEntry(subEntry, pathToSend);
				}
			} while (entries.length > 0);
		}
	}

	// TODO: How do we want to render rudimentary metrics?
	function enableStatWatch() {
		// Clear any previous stats loop to prevent timer leaks on reconnect
		if (statsLoopId) {
			clearInterval(statsLoopId);
			statsLoopId = null;
		}
		// Start watching stats
		var videoBytesReceivedStart = 0;
		var audioBytesReceivedStart = 0;
		var previousVideoJitterBufferDelay = 0.0;
		var previousVideoJitterBufferEmittedCount = 0;
		var previousAudioJitterBufferDelay = 0.0;
		var previousAudioJitterBufferEmittedCount = 0;
		var statsStart = new Date().getTime() / 1000;
		statsLoopId = setInterval(async () => {
			webrtc.getConnectionStats().then((stats) => {
				statWatchEnabled = true;
				var now = new Date().getTime() / 1000;
				connectionStat = {};

				// Connection latency in milliseconds
				const rtt = (stats.general.currentRoundTripTime !== null) ? (stats.general.currentRoundTripTime * 1000.0) : (serverLatency)

				// Connection stats
				connectionStat.connectionPacketsReceived = stats.general.packetsReceived;
				connectionStat.connectionPacketsLost = stats.general.packetsLost;
				connectionStat.connectionStatType = stats.general.connectionType

				var connEl = document.getElementById('conn-indicator');
				if (connEl) {
					var ct = stats.general.connectionType;
					if (ct === 'relay') {
						connEl.textContent = 'RELAY';
						connEl.style.color = '#f0a020';
					} else if (ct === 'host') {
						connEl.textContent = 'TCP';
						connEl.style.color = '#4caf50';
					} else if (ct && ct !== 'NA' && ct !== 'unknown') {
						connEl.textContent = ct.toUpperCase();
						connEl.style.color = '#4caf50';
					} else {
						connEl.textContent = '—';
						connEl.style.color = 'rgba(255, 255, 255, 0.5)';
					}
					connEl.title = '连接模式: ' + (ct || 'unknown');
				}

				connectionStat.connectionBytesReceived = (stats.general.bytesReceived * 1e-6).toFixed(2) + " MBytes";
				connectionStat.connectionBytesSent = (stats.general.bytesSent * 1e-6).toFixed(2) + " MBytes";
				connectionStat.connectionAvailableBandwidth = (parseInt(stats.general.availableReceiveBandwidth) / 1e+6).toFixed(2) + " mbps";

				// Video stats
				connectionStat.connectionCodec = stats.video.codecName;
				connectionStat.connectionVideoDecoder = stats.video.decoder;
				connectionStat.connectionResolution = stats.video.frameWidth + "x" + stats.video.frameHeight;
				connectionStat.connectionFrameRate = stats.video.framesPerSecond;
				connectionStat.connectionVideoBitrate = (((stats.video.bytesReceived - videoBytesReceivedStart) / (now - statsStart)) * 8 / 1e+6).toFixed(2);
				videoBytesReceivedStart = stats.video.bytesReceived;

				// Audio stats
				connectionStat.connectionAudioCodecName = stats.audio.codecName;
				connectionStat.connectionAudioBitrate = (((stats.audio.bytesReceived - audioBytesReceivedStart) / (now - statsStart)) * 8 / 1e+3).toFixed(2);
				audioBytesReceivedStart = stats.audio.bytesReceived;

				// Latency stats
				connectionStat.connectionVideoLatency = parseInt(Math.round(rtt + (1000.0 * (stats.video.jitterBufferDelay - previousVideoJitterBufferDelay) / (stats.video.jitterBufferEmittedCount - previousVideoJitterBufferEmittedCount) || 0)));
				previousVideoJitterBufferDelay = stats.video.jitterBufferDelay;
				previousVideoJitterBufferEmittedCount = stats.video.jitterBufferEmittedCount;
				connectionStat.connectionAudioLatency = parseInt(Math.round(rtt + (1000.0 * (stats.audio.jitterBufferDelay - previousAudioJitterBufferDelay) / (stats.audio.jitterBufferEmittedCount - previousAudioJitterBufferEmittedCount) || 0)));
				previousAudioJitterBufferDelay = stats.audio.jitterBufferDelay;
				previousAudioJitterBufferEmittedCount = stats.audio.jitterBufferEmittedCount;

				// Format latency
				connectionStat.connectionLatency =  Math.max(connectionStat.connectionVideoLatency, connectionStat.connectionAudioLatency);

				statsStart = now;
				window.fps = connectionStat.connectionFrameRate

				if (webrtc._send_channel !== null && webrtc._send_channel.readyState === 'open') {
					// Send compact stats summary instead of full allReports
					// (allReports can be 5-15KB, exceeding DTLS/SCTP frame limits)
					var summary = {
						video: {
							bytesReceived: stats.video.bytesReceived,
							packetsReceived: stats.video.packetsReceived,
							packetsLost: stats.video.packetsLost,
							framesPerSecond: stats.video.framesPerSecond,
							frameWidth: stats.video.frameWidth,
							frameHeight: stats.video.frameHeight,
							codecName: stats.video.codecName,
							decoder: stats.video.decoder,
						},
						audio: {
							bytesReceived: stats.audio.bytesReceived,
							packetsReceived: stats.audio.packetsReceived,
							packetsLost: stats.audio.packetsLost,
							codecName: stats.audio.codecName,
						},
						general: {
							bytesReceived: stats.general.bytesReceived,
							bytesSent: stats.general.bytesSent,
							currentRoundTripTime: stats.general.currentRoundTripTime,
							connectionType: stats.general.connectionType,
						}
					};
					webrtc.sendDataChannelMessage(`_stats_video,${JSON.stringify(summary)}`);
				}
			});
		// Stats refresh interval (1000 ms)
		}, 1000);
	}

	function handleWindowFocus() {
		if (webrtc._send_channel === null || webrtc._send_channel.readyState !== 'open') return;
		// reset keyboard to avoid stuck keys.
		webrtc.sendDataChannelMessage("kr");
		// clipboard interface is only available in secure context
		if (window.isSecureContext) {
			// Send clipboard contents.
			navigator.clipboard.readText()
				.then(text => {
						webrtc.sendDataChannelMessage(`cw,${stringToBase64(text)}`);
				})
				.catch(err => {
						webrtc._setStatus('Failed to read clipboard contents: ' + err);
				});
		}
	}

	function handleWindowBlur() {
		if (webrtc._send_channel === null || webrtc._send_channel.readyState !== 'open') return;
		// reset keyboard to avoid stuck keys.
		webrtc.sendDataChannelMessage("kr");
	}

	function setupKeyBoardAssisstant() {
		const keyboardInputAssist = document.getElementById('keyboard-input-assist');
		if (keyboardInputAssist && input) {
			keyboardInputAssist.addEventListener('input', (event) => {
				const typedString = keyboardInputAssist.value;
				if (typedString) {
				input._typeString(typedString);
				keyboardInputAssist.value = '';
				}
			});
		keyboardInputAssist.addEventListener('keydown', (event) => {
			if (event.key === 'Enter' || event.keyCode === 13) {
			const enterKeysym = 0xFF0D;
			input._guac_press(enterKeysym);
			setTimeout(() => input._guac_release(enterKeysym), 5);
			event.preventDefault();
			keyboardInputAssist.value = '';
			} else if (event.key === 'Backspace' || event.keyCode === 8) {
			const backspaceKeysym = 0xFF08;
			input._guac_press(backspaceKeysym);
			setTimeout(() => input._guac_release(backspaceKeysym), 5);
			event.preventDefault();
			}
		});
		console.log("Added 'input' and 'keydown' listeners to #keyboard-input-assist.");
		} else {
		console.error(" Could not add listeners to keyboard assist: Element or Input handler instance not found.");
		}
	}

	return {
		initialize() {
			InitUI();
			// Create the nodes and configure its attributes
			const appDiv = document.getElementById('app');
			let videoContainer = document.createElement("div");
			videoContainer.className = "video-container";

			playButtonElement = document.createElement('button');
			playButtonElement.id = 'playButton';
			playButtonElement.textContent = 'Play Stream';
			playButtonElement.classList.add('hidden');
			playButtonElement.addEventListener("click", playStream);

			statusDisplayElement = document.createElement('div');
			statusDisplayElement.id = 'status-display';
			statusDisplayElement.className = 'status-bar';
			statusDisplayElement.textContent = 'Connecting...';

			let overlayInput = document.createElement('input');
			overlayInput.type = 'text';
			overlayInput.readOnly = true;
			overlayInput.id = 'overlayInput';

			// prepare the video and audio elements
			videoElement = document.createElement('video');
			videoElement.id = 'stream';
			videoElement.className = 'video';
			videoElement.autoplay = true;
			videoElement.playsInline = true;
			videoElement.muted = true;
			videoElement.setAttribute('muted', '');
			videoElement.setAttribute('autoplay', '');
			videoElement.setAttribute('playsinline', '');
			// NOTE: Do NOT set contentEditable on the video element.
			// When a system IME is active (even in English mode), contentEditable
			// causes the browser to report keyCode===229 for ALL keystrokes,
			// which breaks keyboard input entirely.

			const hiddenFileInput = document.createElement('input');
			hiddenFileInput.type = 'file';
			hiddenFileInput.id = 'globalFileInput';
			hiddenFileInput.multiple = true;
			hiddenFileInput.style.display = 'none';
			document.body.appendChild(hiddenFileInput);
			hiddenFileInput.addEventListener('change', handleFileInputChange);

			videoContainer.appendChild(videoElement);
			videoContainer.appendChild(playButtonElement);

			// No-window overlay (shown when no X11 windows are running)
			const noWindowOverlay = document.createElement('div');
			noWindowOverlay.className = 'no-window-overlay hidden';
			noWindowOverlay.innerHTML = '<div class="no-window-content"><h2>等待应用启动</h2><p>当前没有应用在运行</p></div>';
			videoContainer.appendChild(noWindowOverlay);

			// Taskbar: trigger zone + bar
			const taskbarTrigger = document.createElement('div');
			taskbarTrigger.className = 'taskbar-trigger';
			const taskbar = document.createElement('div');
			taskbar.className = 'taskbar';
			taskbar.id = 'taskbar';

			// Pin button (first element in taskbar)
			let taskbarPinned = getBoolParam('taskbar_pinned', true);
			// SVG icons for pin states
			// Unpinned: tilted pin (📌 style)
			const pinSvgUnpinned = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><g transform="rotate(-45 12 12)"><path d="M12 17v5"/><path d="M9 11V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v7"/><path d="M5 15h14"/><path d="M9 11l-2 4h10l-2-4"/></g></svg>`;
			// Pinned: straight down pin
			const pinSvgPinned = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 17v5"/><path d="M9 11V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v7"/><path d="M5 15h14"/><path d="M9 11l-2 4h10l-2-4"/></svg>`;

			const pinBtn = document.createElement('div');
			pinBtn.className = 'taskbar-pin' + (taskbarPinned ? ' active' : '');
			pinBtn.innerHTML = taskbarPinned ? pinSvgPinned : pinSvgUnpinned;
			pinBtn.title = taskbarPinned ? '取消固定' : '固定任务栏';
			if (taskbarPinned) taskbar.classList.add('pinned');
			pinBtn.addEventListener('click', (e) => {
				e.stopPropagation();
				taskbarPinned = !taskbarPinned;
				pinBtn.classList.toggle('active', taskbarPinned);
				taskbar.classList.toggle('pinned', taskbarPinned);
				pinBtn.innerHTML = taskbarPinned ? pinSvgPinned : pinSvgUnpinned;
				pinBtn.title = taskbarPinned ? '取消固定' : '固定任务栏';
				setBoolParam('taskbar_pinned', taskbarPinned);
			});
			taskbar.appendChild(pinBtn);

			// IME toggle button
			let imeModeActive = getBoolParam('ime_mode', false);
			const imeSvg = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="4" width="20" height="16" rx="2"/><text x="12" y="16" text-anchor="middle" font-size="11" fill="currentColor" stroke="none" font-weight="bold">中</text></svg>`;
			const imeBtn = document.createElement('div');
			imeBtn.className = 'taskbar-pin' + (imeModeActive ? ' active' : '');
			imeBtn.innerHTML = imeSvg;
			imeBtn.title = imeModeActive ? '关闭中文输入 (Ctrl+Shift+Space)' : '中文输入 (Ctrl+Shift+Space)';
			imeBtn.addEventListener('click', (e) => {
				e.stopPropagation();
				if (input) {
					const active = input.toggleImeMode();
					imeBtn.classList.toggle('active', active);
					imeBtn.title = active ? '关闭中文输入 (Ctrl+Shift+Space)' : '中文输入 (Ctrl+Shift+Space)';
					setBoolParam('ime_mode', active);
				}
			});
			taskbar.appendChild(imeBtn);

			// Change password button
			const pwdSvg = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="11" width="18" height="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>`;
			const pwdBtn = document.createElement('div');
			pwdBtn.className = 'taskbar-pin';
			pwdBtn.innerHTML = pwdSvg;
			pwdBtn.title = '修改密码';
			pwdBtn.addEventListener('click', (e) => {
				e.stopPropagation();
				showChangePasswordModal();
			});
			taskbar.appendChild(pwdBtn);

			const connIndicator = document.createElement('div');
			connIndicator.className = 'taskbar-conn';
			connIndicator.id = 'conn-indicator';
			connIndicator.textContent = '—';
			connIndicator.title = '连接模式';
			taskbar.appendChild(connIndicator);

			document.body.appendChild(taskbarTrigger);
			document.body.appendChild(taskbar);

			let taskbarHideTimer = null;
			const showTaskbar = () => {
				clearTimeout(taskbarHideTimer);
				taskbar.classList.add('visible');
			};
			const hideTaskbar = () => {
				if (taskbarPinned) return;
				taskbarHideTimer = setTimeout(() => {
					taskbar.classList.remove('visible');
				}, 400);
			};
			// overlayInput captures all pointer events, so taskbar-trigger
			// never receives mouseenter. Detect bottom-edge hover via
			// mousemove on the document instead.
			document.addEventListener('mousemove', (e) => {
				if (e.clientY >= window.innerHeight - 6) {
					showTaskbar();
				} else if (e.clientY < window.innerHeight - 42) {
					hideTaskbar();
				}
			});
			taskbar.addEventListener('mouseenter', showTaskbar);
			taskbar.addEventListener('mouseleave', hideTaskbar);

			videoContainer.appendChild(statusDisplayElement);
			videoContainer.appendChild(overlayInput);
			appDiv.appendChild(videoContainer);

			if (!document.getElementById('keyboard-input-assist')) {
				const keyboardInputAssist = document.createElement('input');
				keyboardInputAssist.type = 'text';
				keyboardInputAssist.id = 'keyboard-input-assist';
				keyboardInputAssist.style.position = 'absolute';
				keyboardInputAssist.style.left = '-9999px';
				keyboardInputAssist.style.top = '-9999px';
				keyboardInputAssist.style.width = '1px';
				keyboardInputAssist.style.height = '1px';
				keyboardInputAssist.style.opacity = '0';
				keyboardInputAssist.style.border = '0';
				keyboardInputAssist.style.padding = '0';
				keyboardInputAssist.style.caretColor = 'transparent';
				keyboardInputAssist.setAttribute('aria-hidden', 'true');
				keyboardInputAssist.setAttribute('autocomplete', 'off');
				keyboardInputAssist.setAttribute('autocorrect', 'off');
				keyboardInputAssist.setAttribute('autocapitalize', 'off');
				keyboardInputAssist.setAttribute('spellcheck', 'false');
				document.body.appendChild(keyboardInputAssist);
				console.log("Dynamically added #keyboard-input-assist element.");
			}
			// Fetch locally stored application data
			appName = window.location.pathname.endsWith("/") && (window.location.pathname.split("/")[1]) || "webrtc";
			debug = getBoolParam('debug', false);
			setBoolParam('debug', debug);
			// TCP-only: ignore legacy relay switch setting entirely.
			resizeRemote = getBoolParam('resize_remote', resizeRemote);
			setBoolParam('resize_remote', resizeRemote)
			scaleLocal = getBoolParam('scaleLocallyManual', !resizeRemote);
			setBoolParam('scaleLocallyManual', scaleLocal);
			videoBitRate = getIntParam('video_bitrate', videoBitRate);
			setIntParam('video_bitrate', videoBitRate);
			videoFramerate = getIntParam('framerate', videoFramerate);
			setIntParam('framerate', videoFramerate);
			audioBitRate = getIntParam('audio_bitrate', audioBitRate);
			setIntParam('audio_bitrate', audioBitRate);
			window.isManualResolutionMode = getBoolParam('is_manual_resolution_mode', false);
			setBoolParam('is_manual_resolution_mode', window.isManualResolutionMode);
			manualWidth = getIntParam('manual_width', null);
			setIntParam('manual_width', manualWidth);
			manualHeight = getIntParam('manual_height', null);
			setIntParam('manual_height', manualHeight);
			encoder = getStringParam('encoder_rtc', 'x264enc');
			setStringParam('encoder_rtc', encoder)
			useCssScaling = getBoolParam('useCssScaling', true);  // TODO: need to handle hiDPI
			setBoolParam('useCssScaling', useCssScaling);

			// listen for dashboard messages (Dashboard -> core client)
			window.addEventListener("message", handleMessage);
			// listen for file upload event
			window.addEventListener('requestFileUpload', handleRequestFileUpload);
			// handlers to handle the drop in files/directories for upload
			overlayInput.addEventListener('dragover', handleDragOver);
			overlayInput.addEventListener('drop', handleDrop);

			// WebRTC entrypoint, connect to the signaling server
			var pathname = window.location.pathname;
			pathname = pathname.slice(0, pathname.lastIndexOf("/") + 1);
			var protocol = (location.protocol == "http:" ? "ws://" : "wss://");
			var signaling = new WebRTCDemoSignaling(new URL(protocol + window.location.host + pathname + appName + "/signaling/"));
			webrtc = new WebRTCDemo(signaling, videoElement, 1);
			const send = (data) => {
				webrtc.sendDataChannelMessage(data);
			}
			input = new Input(overlayInput, send, false, useCssScaling=useCssScaling);

			setupKeyBoardAssisstant();

			// assign the handlers to respective objects
			// TODO: Need to handle the logEntries and DebugEntries list
			signaling.onstatus = (message) => {
				logEntries.push(applyTimestamp("[signaling] " + message));
				console.log("[signaling] " + message);
			};
			signaling.onerror = (message) => {
				logEntries.push(applyTimestamp("[signaling] [ERROR] " + message))
				console.log("[signaling ERROR] " + message);
			};

			signaling.ondisconnect = (reconnect) => {
				videoElement.style.cursor = "auto";
				if (reconnect) {
					// If WebRTC media is already flowing, don't tear it down
					// just because the signaling WebSocket was closed by a proxy.
					// Only reset when the peer connection is actually dead.
					var pc = webrtc.peerConnection;
					if (pc && (pc.connectionState === 'connected' || pc.connectionState === 'connecting')) {
						console.log("[signaling] WebSocket closed but WebRTC still alive, reconnecting signaling only");
						status = 'connected';
						signaling.connect();
					} else {
						status = 'connecting';
						webrtc.reset();
					}
				} else {
					status = 'disconnected';
				}
				updateStatusDisplay();
			};

			// Send webrtc status and error messages to logs.
			webrtc.onstatus = (message) => {
				logEntries.push(applyTimestamp("[webrtc] " + message));
				console.log("[webrtc] " + message);
			};
			webrtc.onerror = (message) => {
				logEntries.push(applyTimestamp("[webrtc] [ERROR] " + message));
				console.log("[webrtc] [ERROR] " + message);
			};

			if (debug) {
				signaling.ondebug = (message) => { debugEntries.push("[signaling] " + message); };
				webrtc.ondebug = (message) => { debugEntries.push(applyTimestamp("[webrtc] " + message)) };
			}

			webrtc.ongpustats = async (stats) => {
				// Gpu stats for the Dashboard to render
				window.gpu_stats = stats;
			}

			webrtc.onconnectionstatechange = (state) => {
				videoConnected = state;
				if (videoConnected === "connected") {
					// Repeatedly emit minimum latency target
					webrtc.peerConnection.getReceivers().forEach((receiver) => {
						let intervalLoop = setInterval(async () => {
							if (receiver.track.readyState !== "live" || receiver.transport.state !== "connected") {
								clearInterval(intervalLoop);
								return;
							} else {
								receiver.jitterBufferTarget = receiver.jitterBufferDelayHint = receiver.playoutDelayHint = 0;
							}
						}, 15);
					});
					status = state;
					if (!statWatchEnabled) {
						enableStatWatch();
					}
				} else if (videoConnected === "failed") {
					// WebRTC connection died — reset and reconnect
					console.log("[webrtc] Connection failed, resetting");
					status = 'connecting';
					webrtc.reset();
				}
				updateStatusDisplay();
			};

			webrtc.ondatachannelopen = () => {
				console.log("Data channel opened");
				// Bind input handlers.
				input.attach();
				loadLastSessionSettings();
				sendClientPersistedSettings();

				// Restore IME mode from localStorage
				if (imeModeActive && input) {
					input.toggleImeMode();
				}

				// Send client-side metrics over data channel every 5 seconds
				if (metricsLoopId) {
					clearInterval(metricsLoopId);
				}
				metricsLoopId = setInterval(async () => {
					if (connectionStat.connectionFrameRate === parseInt(connectionStat.connectionFrameRate, 10))webrtc.sendDataChannelMessage(`_f,${connectionStat.connectionFrameRate}`);
					if (connectionStat.connectionLatency === parseInt(connectionStat.connectionLatency, 10)) webrtc.sendDataChannelMessage(`_l,${connectionStat.connectionLatency}`);
				}, 5000)
			}

			webrtc.ondatachannelclose = () => {
				input.detach();
			}

			input.onmenuhotkey = () => {
				showDrawer = !showDrawer;
			}

			input.onimetoggle = (active) => {
				imeBtn.classList.toggle('active', active);
				imeBtn.title = active ? '关闭中文输入 (Ctrl+Shift+Space)' : '中文输入 (Ctrl+Shift+Space)';
				setBoolParam('ime_mode', active);
			};

			webrtc.onplaystreamrequired = () => {
				// Auto-retry with muted to bypass autoplay policy
				if (videoElement && videoElement.paused) {
					videoElement.muted = true;
					videoElement.setAttribute('muted', '');
					videoElement.play().then(() => {
						showStart = false;
						if (playButtonElement) playButtonElement.classList.add('hidden');
						webrtc.unmuteAudio();
					}).catch(() => {
						showStart = true;
					});
				} else {
					showStart = true;
				}
			}

			// Unmute audio on first user interaction (click/keydown)
			const unmuteOnInteraction = () => {
				webrtc.unmuteAudio();
				document.removeEventListener('click', unmuteOnInteraction);
				document.removeEventListener('keydown', unmuteOnInteraction);
			};
			document.addEventListener('click', unmuteOnInteraction);
			document.addEventListener('keydown', unmuteOnInteraction);

			// Actions to take whenever window changes focus
			window.addEventListener('focus', handleWindowFocus);
			window.addEventListener('blur', handleWindowBlur);

			// --- Clipboard sync ---
			// Hidden textarea for programmatic clipboard operations
			var _clipTA = document.createElement('textarea');
			_clipTA.style.cssText = 'position:fixed;left:-9999px;top:-9999px;width:1px;height:1px;opacity:0.01;';
			_clipTA.id = '_ivnc_clip';
			document.body.appendChild(_clipTA);

			var _programmaticCopy = false;

			webrtc.onclipboardcontent = (content) => {
				if (clipboardStatus !== 'enabled') return;
				webrtc._remoteClipboard = content;
				if (navigator.clipboard && navigator.clipboard.writeText) {
					navigator.clipboard.writeText(content).catch(() => {
						doCopyToSystem(content);
					});
				} else {
					doCopyToSystem(content);
				}
			}

			function doCopyToSystem(text) {
				_clipTA.value = text;
				_clipTA.select();
				_programmaticCopy = true;
				document.execCommand('copy');
				_programmaticCopy = false;
				var ime = document.querySelector('textarea[autocomplete="off"]');
				if (ime) ime.focus({ preventScroll: true });
			}

			// Expose clipboard helpers globally for input2.js to call
			var _lastPasteTime = 0;
			var _overlayRef = document.getElementById('overlayInput');
			window.__ivncClipboard = {
				enablePaste: function() {
					var now = Date.now();
					if (now - _lastPasteTime < 1000) return; // Throttle here
					if (!_overlayRef) _overlayRef = document.getElementById('overlayInput');
					if (_overlayRef) {
						_overlayRef.readOnly = false;
						_overlayRef.value = '';
					}
				}
			};

			// Paste event handler — works when overlayInput is temporarily editable
			document.addEventListener('paste', (e) => {
				// Restore readonly immediately
				if (_overlayRef) {
					_overlayRef.readOnly = true;
					_overlayRef.value = '';
				}
				if (clipboardStatus !== 'enabled') return;
				var now = Date.now();
				if (now - _lastPasteTime < 1000) {
					e.preventDefault();
					return;
				}
				_lastPasteTime = now;
				var text = e.clipboardData && e.clipboardData.getData('text/plain');
				if (text) {
					e.preventDefault();
					webrtc.sendDataChannelMessage('cw,' + stringToBase64(text));
				}
			});

			webrtc.oncursorchange = (cursorData) => {
				input.updateServerCursor(cursorData);
			}

			webrtc.ontaskbarupdate = (data) => {
				const tb = document.getElementById('taskbar');
				if (!tb) return;
				const wins = data.windows || [];
				// Remove all items except the pin button
				Array.from(tb.querySelectorAll('.taskbar-item')).forEach(el => el.remove());
				// Hide taskbar when no windows
				if (wins.length === 0) {
					tb.classList.remove('visible');
					return;
				}
				wins.forEach((w) => {
					const item = document.createElement('div');
					item.className = 'taskbar-item' + (w.focused ? ' focused' : '');
					item.title = `${w.title} (${w.app_id})`;

					// Label with title (truncate if too long)
					const label = document.createElement('span');
					label.textContent = w.display_name || w.app_id || w.title || `Window ${w.id}`;
					label.style.textOverflow = 'ellipsis';
					label.style.overflow = 'hidden';
					label.style.whiteSpace = 'nowrap';
					label.style.maxWidth = '160px';
					item.appendChild(label);

					// Close button (only for focused item)
					if (w.focused) {
						const closeBtn = document.createElement('span');
						closeBtn.className = 'taskbar-close';
						closeBtn.textContent = '✕';
						closeBtn.title = '关闭应用';
						closeBtn.addEventListener('click', (e) => {
							e.stopPropagation();
							webrtc.sendDataChannelMessage(`close,${w.id}`);
						});
						item.appendChild(closeBtn);
					}

					// Click to focus
					item.addEventListener('click', (e) => {
						if (e.target.className === 'taskbar-close') return;
						e.stopPropagation();
						webrtc.sendDataChannelMessage(`focus,${w.id}`);
					});
					tb.appendChild(item);
				});
			}

			webrtc.onsystemaction = (action) => {
				webrtc._setStatus("Executing system action: " + action);
				if (action === 'reload') {
					setTimeout(() => {
						// trigger webrtc.reset() by disconnecting from the signaling server.
						signaling.disconnect();
					}, 700);
				} else {
					webrtc._setStatus('Server sent acknowledgement for ' + action);
				}
			}

			webrtc.onlatencymeasurement = (latency_ms) => {
				serverLatency = latency_ms * 2.0;
			}

			webrtc.onsystemstats = async (stats) => {
				// Dashboard takes care of data validation
				window.system_stats = stats;
			}

			webrtc.onserversettings = (obj) => {
				console.log("Received server settings payload:", obj.settings);
				const changes = sanitizeAndStoreSettings(obj.settings);
				window.postMessage({ type: 'serverSettings', payload: obj.settings }, window.location.origin);
				if (Object.keys(changes).length > 0) {
						// TODO: server-side handling of settings updates
						// console.log('Client settings were sanitized by server rules. Sending updates back to server:', changes);
						handleSettingsMessage(changes);
				}
				if (obj.settings && obj.settings.is_manual_resolution_mode && obj.settings.is_manual_resolution_mode.value === true) {
					console.log("Server settings payload confirms manual mode. Switching to manual resize handlers.");
					const serverWidth = obj.settings.manual_width ? parseInt(obj.settings.manual_width.value, 10) : 0;
					const serverHeight = obj.settings.manual_height ? parseInt(obj.settings.manual_height.value, 10) : 0;
					if (serverWidth > 0 && serverHeight > 0) {
							console.log(`Applying server-enforced manual resolution: ${serverWidth}x${serverHeight}`);
							window.is_manual_resolution_mode = true;
							manualWidth = serverWidth;
							manualHeight = serverHeight;
							applyManualStyle(manualWidth, manualHeight, scaleLocal);
					} else {
							console.warn("Server dictated manual mode but did not provide valid dimensions.");
					}
					disableAutoResize();
				} else {
						console.log("Server settings payload confirms auto mode. Switching to auto resize handlers.");
						enableAutoResize();
				}
			}

			// Safari without Permission API enabled fails
			if (navigator.permissions) {
				navigator.permissions.query({
					name: 'clipboard-read'
				}).then(permissionStatus => {
					// Will be 'granted', 'denied' or 'prompt':
					if (permissionStatus.state === 'granted') {
							clipboardStatus = 'enabled';
					}

					// Listen for changes to the permission state
					permissionStatus.onchange = () => {
							if (permissionStatus.state === 'granted') {
									clipboardStatus = 'enabled';
							}
					};
				});
			}

			// TCP-only: directly connect using the SDP answer's TCP candidate.
			windowResolution = input.getWindowResolution();
			signaling.currRes = windowResolution;
			webrtc.connect();
		},
		cleanup() {
			// reset the data
			window.isManualResolutionMode = false;
			window.fps = 0;

			// remove the listeners
			window.removeEventListener("message", handleMessage);
			window.removeEventListener("resize", resizeStart);
			window.removeEventListener("requestFileUpload", handleRequestFileUpload);
			window.removeEventListener("focus", handleWindowFocus);
			window.removeEventListener("blur", handleWindowBlur);

			// temporary workaround to nullify/reset the variables
			appName = null;
			videoBitRate = 8000;
			videoFramerate = 60;
			audioBitRate = 128000;
			showStart = false;
			showDrawer = false;
			logEntries = [];
			debugEntries = [];
			status = 'connecting';
			clipboardStatus = 'enabled';
			windowResolution = "";
			encoderLabel = "";
			encoder = ""
			connectionStat = {
					connectionStatType: "unknown",
					connectionLatency: 0,
					connectionVideoLatency: 0,
					connectionAudioLatency: 0,
					connectionAudioCodecName: "NA",
					connectionAudioBitrate: 0,
					connectionPacketsReceived: 0,
					connectionPacketsLost: 0,
					connectionBytesReceived: 0,
					connectionBytesSent: 0,
					connectionCodec: "unknown",
					connectionVideoDecoder: "unknown",
					connectionResolution: "",
					connectionFrameRate: 0,
					connectionVideoBitrate: 0,
					connectionAvailableBandwidth: 0
			};
			serverLatency = 0;
			resizeRemote = false;
			scaleLocal = false;
			debug = false;
			playButtonElement = null;
			statusDisplayElement = null;
			rtime = null;
			rdelta = 500;
			rtimeout = false;
			manualWidth = manualHeight = 0;
			videoConnected = "";
			audioConnected = "";
			statWatchEnabled = false;
			if (statsLoopId) { clearInterval(statsLoopId); statsLoopId = null; }
			if (metricsLoopId) { clearInterval(metricsLoopId); metricsLoopId = null; }
			webrtc = null;
			input = null;
			useCssScaling = true;
		}
	}
}
