import { useEffect, useRef, useState } from 'react';
import './RemoteDesktop.css';

interface RemoteDesktopProps {
  params?: {
    serverUrl?: string;
  };
}

interface ServerInfo {
  supports_webrtc: boolean;
  screen_width: number;
  screen_height: number;
}

type ConnectionState = 'connecting' | 'connected' | 'disconnected' | 'error';

export const RemoteDesktopPanel = (props: RemoteDesktopProps) => {
  const videoRef = useRef<HTMLVideoElement>(null);
  const pcRef = useRef<RTCPeerConnection | null>(null);
  const [connectionState, setConnectionState] = useState<ConnectionState>('connecting');
  const [serverInfo, setServerInfo] = useState<ServerInfo | null>(null);
  const [sessionId, setSessionId] = useState<string>('');

  const serverUrl = props.params?.serverUrl || `${window.location.protocol}//${window.location.host}`;

  // 获取服务器信息并创建会话
  useEffect(() => {
    const initWebRTC = async () => {
      try {
        // 1. 获取服务器信息
        const infoRes = await fetch(`${serverUrl}/api/rdp/info`);
        const info = await infoRes.json();
        setServerInfo(info);
        console.log('Server info:', info);

        // 2. 创建会话
        const sessionRes = await fetch(`${serverUrl}/api/rdp/session`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ fps: 30 }),
        });
        const sessionData = await sessionRes.json();
        
        if (!sessionData.session_id) {
          throw new Error('Failed to create session');
        }
        
        setSessionId(sessionData.session_id);
        console.log('Session created:', sessionData.session_id);

        // 3. 创建 WebRTC 连接
        await createPeerConnection(sessionData.session_id);
      } catch (error) {
        console.error('Failed to initialize WebRTC:', error);
        setConnectionState('error');
      }
    };

    initWebRTC();

    return () => {
      cleanup();
    };
  }, [serverUrl]);

  const createPeerConnection = async (sid: string) => {
    try {
      // 创建 RTCPeerConnection
      const pc = new RTCPeerConnection({
        iceServers: [
          { urls: 'stun:stun.l.google.com:19302' },
          { urls: 'stun:stun1.l.google.com:19302' },
        ],
      });
      pcRef.current = pc;
      
      // 暴露到window用于调试
      (window as any).webrtcPC = pc;

      // 处理远程视频流
      pc.ontrack = (event) => {
        console.log('Received remote track:', event.streams);
        if (videoRef.current && event.streams[0]) {
          videoRef.current.srcObject = event.streams[0];
          setConnectionState('connected');
        }
      };

      // 监听连接状态
      pc.onconnectionstatechange = () => {
        console.log('Connection state:', pc.connectionState);
        if (pc.connectionState === 'connected') {
          setConnectionState('connected');
          // 开始收集统计信息
          startStatsCollection(pc);
        } else if (pc.connectionState === 'failed' || pc.connectionState === 'closed') {
          setConnectionState('disconnected');
        }
      };

      pc.oniceconnectionstatechange = () => {
        console.log('ICE connection state:', pc.iceConnectionState);
      };

      // 收集 ICE candidate
      const iceCandidates: RTCIceCandidate[] = [];
      pc.onicecandidate = (event) => {
        if (event.candidate) {
          iceCandidates.push(event.candidate);
          console.log('ICE candidate:', event.candidate.candidate);
        }
      };

      // 创建 DataChannel 用于输入控制
      const dataChannel = pc.createDataChannel('input', { ordered: true });
      dataChannel.onopen = () => console.log('DataChannel opened');
      dataChannel.onclose = () => console.log('DataChannel closed');

      // 添加视频接收轨道
      pc.addTransceiver('video', { direction: 'recvonly' });

      // 创建 Offer
      const offer = await pc.createOffer();
      await pc.setLocalDescription(offer);

      console.log('Created offer:', offer.sdp?.substring(0, 100));

      // 发送 Offer 到服务器
      const offerRes = await fetch(`${serverUrl}/api/rdp/offer`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          sdp: offer.sdp,
          session_id: sid,
        }),
      });

      const answerData = await offerRes.json();
      if (!answerData.success) {
        throw new Error('Failed to get answer from server');
      }

      console.log('Received answer');

      // 设置远程描述
      await pc.setRemoteDescription(new RTCSessionDescription({
        type: 'answer',
        sdp: answerData.sdp,
      }));

      // 发送 ICE candidates
      setTimeout(async () => {
        for (const candidate of iceCandidates) {
          await fetch(`${serverUrl}/api/rdp/ice`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
              candidate: candidate.candidate,
              sdpMid: candidate.sdpMid,
              sdpMLineIndex: candidate.sdpMLineIndex,
              session_id: sid,
            }),
          });
        }
      }, 1000);

      // 获取服务器 ICE candidates
      setTimeout(async () => {
        try {
          const candidatesRes = await fetch(`${serverUrl}/api/rdp/ice-candidates`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ session_id: sid }),
          });
          const candidatesData = await candidatesRes.json();
          
          if (candidatesData.success && candidatesData.candidates) {
            for (const cand of candidatesData.candidates) {
              await pc.addIceCandidate(new RTCIceCandidate({
                candidate: cand.candidate,
                sdpMid: cand.sdpMid,
                sdpMLineIndex: cand.sdpMLineIndex,
              }));
            }
          }
        } catch (e) {
          console.error('Failed to get server ICE candidates:', e);
        }
      }, 2000);

    } catch (error) {
      console.error('Failed to create peer connection:', error);
      setConnectionState('error');
    }
  };

  const startStatsCollection = (pc: RTCPeerConnection) => {
    const collectStats = async () => {
      try {
        const stats = await pc.getStats();
        const videoStats: any[] = [];
        const codecStats: any[] = [];
        
        stats.forEach((stat) => {
          if (stat.type === 'inbound-rtp' && stat.kind === 'video') {
            videoStats.push({
              timestamp: stat.timestamp,
              bytesReceived: stat.bytesReceived,
              packetsReceived: stat.packetsReceived,
              packetsLost: stat.packetsLost,
              framesDecoded: stat.framesDecoded,
              framesReceived: stat.framesReceived,
              framesDropped: stat.framesDropped,
              frameWidth: stat.frameWidth,
              frameHeight: stat.frameHeight,
              jitter: stat.jitter,
              codecId: stat.codecId
            });
          }
          if (stat.type === 'codec') {
            codecStats.push({
              id: stat.id,
              mimeType: stat.mimeType,
              clockRate: stat.clockRate,
              sdpFmtpLine: stat.sdpFmtpLine
            });
          }
          if (stat.type === 'track' && stat.kind === 'video') {
            console.log('Track stats:', {
              framesReceived: stat.framesReceived,
              framesDecoded: stat.framesDecoded,
              framesDropped: stat.framesDropped
            });
          }
        });
        
        if (videoStats.length > 0) {
          console.log('Video inbound stats:', videoStats[0]);
        }
        if (codecStats.length > 0) {
          console.log('Codec stats:', codecStats);
        }
        
        (window as any).webrtcStats = { video: videoStats, codecs: codecStats };
      } catch (e) {
        console.error('Failed to get stats:', e);
      }
    };
    
    // 每5秒收集一次统计信息
    const interval = setInterval(collectStats, 5000);
    
    // 立即收集一次
    setTimeout(collectStats, 1000);
    
    // 清理函数
    (window as any).stopStatsCollection = () => clearInterval(interval);
  };

  const cleanup = () => {
    if (pcRef.current) {
      pcRef.current.close();
      pcRef.current = null;
    }
  };

  return (
    <div className="remote-desktop-panel">
      <div className="rdp-toolbar">
        <div className="rdp-toolbar-left">
          <span className={`rdp-status rdp-status-${connectionState}`}>
            {connectionState === 'connecting' && '连接中...'}
            {connectionState === 'connected' && '已连接'}
            {connectionState === 'disconnected' && '已断开'}
            {connectionState === 'error' && '连接错误'}
          </span>
          {serverInfo && (
            <span className="rdp-resolution">
              服务器: {serverInfo.screen_width}x{serverInfo.screen_height}
            </span>
          )}
          {sessionId && (
            <span className="rdp-session-id">
              Session: {sessionId.substring(0, 8)}...
            </span>
          )}
        </div>
      </div>
      
      <div className="rdp-video-container">
        <video
          ref={videoRef}
          autoPlay
          playsInline
          className="rdp-video"
        />
      </div>
      
      <div className="rdp-instructions">
        <p>WebRTC 远程桌面 - 使用 video 元素显示</p>
      </div>
    </div>
  );
};

export default RemoteDesktopPanel;
